//! Serialize IPC away from the Win32 message loop. A stalled daemon must not
//! prevent Explorer callbacks, painting, menu dismissal, or tray shutdown.
use crate::model::Result;
#[cfg(windows)]
use crate::{ipc, store::Paths};
use serde_json::{json, Value};
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub(super) struct Update {
    pub started: Instant,
    pub result: Result<Option<Value>>,
    pub quit: bool,
}

pub(super) struct Backend {
    commands: mpsc::SyncSender<Value>,
    latest: Arc<Mutex<Option<Update>>>,
}

impl Backend {
    #[cfg(windows)]
    pub fn start(paths: Paths) -> Self {
        Self::with_request(move |value, autostart| ipc::request(&paths, value, autostart))
    }

    fn with_request(
        mut request: impl FnMut(Value, bool) -> Result<Option<Value>> + Send + 'static,
    ) -> Self {
        let (commands, receiver) = mpsc::sync_channel::<Value>(16);
        let latest = Arc::new(Mutex::new(None));
        let output = latest.clone();
        thread::spawn(move || {
            let mut next = json!({"op":"status"});
            loop {
                let started = Instant::now();
                let quit = next["op"] == "shutdown";
                let result = request_command(&mut request, next);
                if let Err(error) = &result {
                    eprintln!("托盘后台请求失败：{error}");
                }
                // Keep only the newest status, even while a menu is open. A
                // shutdown acknowledgement stays latched until the UI consumes
                // it, so a failed shutdown cannot leave `quitting` stuck true.
                let finished = quit && result.is_ok();
                {
                    let mut slot = output.lock().unwrap();
                    if quit || !slot.as_ref().is_some_and(|update: &Update| update.quit) {
                        *slot = Some(Update {
                            started,
                            result,
                            quit,
                        });
                    }
                }
                if finished || Arc::strong_count(&output) == 1 {
                    break;
                }
                next = match receiver.recv_timeout(POLL_INTERVAL) {
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout) => json!({"op":"status"}),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
            }
        });
        Self { commands, latest }
    }

    pub fn send(&self, value: Value) -> bool {
        // Bound both memory and repeated media-key actions during an outage.
        self.commands.try_send(value).is_ok()
    }

    pub fn take_update(&self) -> Option<Update> {
        self.latest.lock().unwrap().take()
    }
}

fn request_command(
    request: &mut impl FnMut(Value, bool) -> Result<Option<Value>>,
    mut command: Value,
) -> Result<Option<Value>> {
    let op = command["op"].as_str().unwrap_or_default();
    if matches!(op, "tray_volume" | "tray_mode" | "tray_lyrics_bar") {
        // Resolve relative menu actions in queue order against the daemon, not
        // against a UI snapshot that predates earlier clicks.
        let state = request(json!({"op":"status"}), true)?
            .ok_or_else(|| crate::model::Error::new("后台状态不可用"))?;
        command = match op {
            "tray_volume" => {
                let volume = (state["volume"].as_i64().unwrap_or(60)
                    + command["delta"].as_i64().unwrap_or(0))
                .clamp(0, 100);
                json!({"op":"config","key":"volume","value":volume})
            }
            "tray_mode" => {
                let next = match state["mode"].as_str().unwrap_or("list") {
                    "list" => "repeat",
                    "repeat" => "single",
                    "single" => "shuffle",
                    _ => "list",
                };
                json!({"op":"config","key":"mode","value":next})
            }
            _ => {
                json!({"op":"config","key":"lyrics_bar","value":!state["lyrics_bar"].as_bool().unwrap_or(true)})
            }
        };
    }
    let autostart = command["op"] != "status" && command["op"] != "shutdown";
    request(command, autostart)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapid_relative_actions_accumulate_in_daemon_order() {
        let mut state = json!({"volume":60,"mode":"list","lyrics_bar":true});
        let mut request = |value: Value, _: bool| {
            if value["op"] == "config" {
                state[value["key"].as_str().unwrap()] = value["value"].clone();
            }
            Ok(Some(state.clone()))
        };
        for _ in 0..2 {
            request_command(&mut request, json!({"op":"tray_volume","delta":10})).unwrap();
            request_command(&mut request, json!({"op":"tray_mode"})).unwrap();
            request_command(&mut request, json!({"op":"tray_lyrics_bar"})).unwrap();
        }
        let status = request_command(&mut request, json!({"op":"status"}))
            .unwrap()
            .unwrap();
        assert_eq!(status["volume"], 80);
        assert_eq!(status["mode"], "single");
        assert_eq!(status["lyrics_bar"], true);
        request_command(&mut request, json!({"op":"tray_volume","delta":100})).unwrap();
        let status = request_command(&mut request, json!({"op":"tray_volume","delta":10}))
            .unwrap()
            .unwrap();
        assert_eq!(status["volume"], 100);
    }

    #[test]
    fn failed_shutdown_keeps_worker_available_for_retry() {
        let (entered, calls) = mpsc::channel();
        let mut shutdowns = 0;
        let backend = Backend::with_request(move |value, _| {
            entered.send(value.clone()).unwrap();
            if value["op"] == "shutdown" {
                shutdowns += 1;
                if shutdowns == 1 {
                    return Err(crate::model::Error::new("daemon busy"));
                }
            }
            Ok(None)
        });
        calls.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(backend.send(json!({"op":"shutdown"})));
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap()["op"],
            "shutdown"
        );
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap()["op"],
            "status"
        );
        let failed = backend.take_update().unwrap();
        assert!(failed.quit && failed.result.is_err());
        assert!(backend.send(json!({"op":"shutdown"})));
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap()["op"],
            "shutdown"
        );
        assert!(calls.recv_timeout(Duration::from_secs(2)).is_err());
        let update = backend.take_update().unwrap();
        assert!(update.quit && update.result.is_ok());
    }

    #[test]
    fn blocked_ipc_does_not_block_ui_and_preserves_command_order() {
        let (entered, calls) = mpsc::channel();
        let (release, waiting) = mpsc::channel::<()>();
        let backend = Backend::with_request(move |value, autostart| {
            entered.send((value.clone(), autostart)).unwrap();
            if value["op"] == "status" {
                waiting.recv().unwrap();
            }
            Ok(Some(value))
        });
        assert_eq!(
            calls.recv_timeout(Duration::from_secs(2)).unwrap().0["op"],
            "status"
        );
        assert!(backend.take_update().is_none());
        assert!(backend.send(json!({"op":"next"})));
        assert!(backend.send(json!({"op":"shutdown"})));
        release.send(()).unwrap();
        let (next, autostart) = calls.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(next["op"], "next");
        assert!(autostart);
        let (quit, autostart) = calls.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(quit["op"], "shutdown");
        assert!(!autostart);
        // Receiver disconnect is an acknowledgement that the worker has finished
        // publishing the final update; no timing sleeps are needed.
        assert!(calls.recv_timeout(Duration::from_secs(2)).is_err());
        let update = backend.take_update().unwrap();
        assert!(update.quit);
        assert!(update.started.elapsed() < Duration::from_secs(2));
        assert_eq!(update.result.unwrap().unwrap()["op"], "shutdown");
    }

    #[test]
    fn commands_are_bounded_while_daemon_is_unresponsive() {
        let (entered, calls) = mpsc::channel();
        let (release, waiting) = mpsc::channel::<()>();
        let backend = Backend::with_request(move |_, _| {
            let _ = entered.send(());
            let _ = waiting.recv();
            Ok(None)
        });
        calls.recv_timeout(Duration::from_secs(2)).unwrap();
        for _ in 0..16 {
            assert!(backend.send(json!({"op":"next"})));
        }
        assert!(!backend.send(json!({"op":"next"})));
        // Dropping an idle UI does not join or block on an IPC worker.
        drop(backend);
        drop(calls);
        drop(release);
    }
}
