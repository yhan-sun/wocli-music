use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};
struct App {
    root: tempfile::TempDir,
    bin: PathBuf,
}
impl App {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
            bin: PathBuf::from(env!("CARGO_BIN_EXE_wocli")),
        }
    }
    fn cmd(&self, args: &[&str]) -> std::process::Output {
        Command::new(&self.bin)
            .args(args)
            .env("WOCLI_HOME", self.root.path().join("data"))
            .env("WOCLI_AUDIO_OUTPUT", "null")
            .env("PATH", "/no-python-or-mpv")
            .output()
            .unwrap()
    }
    fn cli(&self, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        let output = self.cmd(&args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn song(&self, name: &str, seconds: u32) -> PathBuf {
        let samples = 8000 * seconds;
        let mut wav = vec![];
        wav.extend(b"RIFF");
        wav.extend((36 + samples * 2).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(8000u32.to_le_bytes());
        wav.extend(16000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((samples * 2).to_le_bytes());
        wav.resize(44 + samples as usize * 2, 0);
        let path = self.root.path().join(name);
        fs::write(&path, wav).unwrap();
        fs::write(path.with_extension("lrc"), "[00:00]第一行\n[00:04]第二行").unwrap();
        path
    }
    fn phase(&self, phase: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let s = self.cli(&["status"]);
            if s["phase"] == phase {
                return s;
            }
            assert!(Instant::now() < deadline, "{s}");
            thread::sleep(Duration::from_millis(30));
        }
    }
    fn socket(&self, value: Value) -> Value {
        let doctor = self.cli(&["doctor"]);
        let paths = wocli_music::store::Paths {
            home: PathBuf::from(doctor["home"].as_str().unwrap()),
            runtime: PathBuf::from(doctor["runtime_dir"].as_str().unwrap()),
        };
        let stream = wocli_music::ipc::connect(&paths).unwrap();
        wocli_music::ipc::exchange(stream, value).unwrap()
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.cmd(&["daemon", "stop"]);
    }
}
#[test]
fn native_service_preserves_pause_seek_lyrics_collections_and_queue() {
    let app = App::new();
    let song = app.song("测试.wav", 30);
    let other = app.song("另一首.wav", 30);
    assert_eq!(app.cli(&["doctor"])["runtime"], "native");
    let state = app.cli(&["play", song.to_str().unwrap()]);
    assert_eq!(state["phase"], "playing");
    let id = state["track"]["id"].as_str().unwrap();
    app.cli(&["pause"]);
    app.cli(&["seek", "5"]);
    let paused = app.cli(&["status"]);
    assert_eq!(paused["phase"], "paused");
    assert!(paused["position"].as_f64().unwrap() >= 5.);
    app.socket(json!({"op":"seek_to","position":15,"track_id":"stale-track"}));
    assert!(app.cli(&["status"])["position"].as_f64().unwrap() < 6.);
    assert!(app.cli(&["lyrics"])["lyrics"]
        .as_str()
        .unwrap()
        .contains("第一行"));
    app.cli(&["like"]);
    app.cli(&["playlist", "create", "通勤"]);
    app.cli(&["playlist", "add", "通勤", "--id", id]);
    app.cli(&["playlist", "rename", "通勤", "日常"]);
    assert_eq!(
        app.cli(&["playlist", "show", "日常"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app.cli(&["playlist", "delete", "日常"]);
    assert_eq!(app.cli(&["favorites"]).as_array().unwrap().len(), 1);
    assert!(song.exists());
    app.cli(&["queue", "add", other.to_str().unwrap()]);
    assert_eq!(app.cli(&["status"])["queue"].as_array().unwrap().len(), 2);
    for mode in ["list", "repeat", "single", "shuffle"] {
        assert_eq!(app.cli(&["mode", mode])["mode"], mode);
    }
    app.cli(&["next"]);
    let next = app.phase("playing");
    assert_ne!(next["track"]["id"], id);
    app.cli(&["prev"]);
    assert_eq!(app.phase("playing")["track"]["id"], id);
    assert_eq!(app.cli(&["lyrics-bar", "status"])["lyrics_bar"], true);
    assert_eq!(app.cli(&["lyrics-bar", "off"])["lyrics_bar"], false);
    assert_eq!(app.cli(&["lyrics-bar", "toggle"])["lyrics_bar"], true);
    assert_eq!(app.cli(&["tray", "status"])["tray_running"], false);
    app.cli(&["daemon", "stop"]);
    let saved = app.cli(&["favorites"]);
    assert_eq!(saved[0]["id"], id);
    app.cli(&["daemon", "start"]);
    assert_eq!(app.cli(&["status"])["queue"].as_array().unwrap().len(), 2);
}
#[test]
fn rejects_invalid_config_and_audio_without_silent_success() {
    let app = App::new();
    assert!(!app.cmd(&["config", "volume", "101"]).status.success());
    assert!(!app.cmd(&["mode", "wrong"]).status.success());
    let bad = app.root.path().join("broken.mp3");
    fs::write(&bad, b"not music").unwrap();
    let output = app.cmd(&["play", bad.to_str().unwrap(), "--json"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("decode_error"));
}

#[test]
fn reads_legacy_negative_queue_index_and_retains_sqlite_data() {
    let app = App::new();
    let home = app.root.path().join("data");
    fs::create_dir_all(&home).unwrap();
    let paths = wocli_music::store::Paths {
        home: home.clone(),
        runtime: home.join("unused"),
    };
    let store = wocli_music::store::Store::open(&paths).unwrap();
    let track = wocli_music::model::Track {
        id: "kuwo:123".into(),
        title: "旧曲库".into(),
        duration: 200.,
        ..Default::default()
    };
    store.favorite(&track).unwrap();
    store.collect("旧收藏夹", &track).unwrap();
    store.played(&track).unwrap();
    store.save_lyric(&track.id, "[00:01]旧歌词").unwrap();
    store
        .save("queue", &json!({"tracks":[],"index":-1}))
        .unwrap();
    fs::write(
        home.join("settings.json"),
        r#"{"quality":"best","volume":70,"mode":"repeat","direct":false,"lyric_offset":250}"#,
    )
    .unwrap();
    assert_eq!(app.cli(&["daemon", "start"])["mode"], "repeat");
    assert_eq!(app.cli(&["favorites"])[0]["title"], "旧曲库");
    assert_eq!(app.cli(&["history"])[0]["id"], track.id);
    assert_eq!(
        app.cli(&["playlist", "show", "旧收藏夹"])[0]["id"],
        track.id
    );
    assert!(app.cli(&["lyrics", "--id", "123"])["lyrics"]
        .as_str()
        .unwrap()
        .contains("旧歌词"));
}

#[test]
fn environment_variables_configure_home_runtime_and_download_dirs() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("custom_home");
    let runtime = root.path().join("custom_runtime");
    let download = root.path().join("custom_downloads");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_wocli"));

    let output = Command::new(&bin)
        .args(["doctor", "--json"])
        .env("WOCLI_HOME", &home)
        .env("WOCLI_RUNTIME_DIR", &runtime)
        .env("WOCLI_DOWNLOAD_DIR", &download)
        .env("WOCLI_AUDIO_OUTPUT", "null")
        .output()
        .unwrap();

    assert!(output.status.success());
    let doctor: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        doctor["home"].as_str().unwrap(),
        fs::canonicalize(&home).unwrap().to_str().unwrap()
    );
    assert_eq!(
        doctor["runtime_dir"].as_str().unwrap(),
        runtime.to_str().unwrap()
    );
    assert_eq!(
        doctor["env"]["WOCLI_DOWNLOAD_DIR"].as_str().unwrap(),
        download.to_str().unwrap()
    );
    assert!(doctor["socket"]
        .as_str()
        .unwrap()
        .starts_with(runtime.to_str().unwrap()));
}
