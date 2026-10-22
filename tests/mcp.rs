use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
use wocli_music::{
    model::Track,
    store::{Paths, Store},
};

struct App {
    root: tempfile::TempDir,
    bin: PathBuf,
}
impl App {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("data")).unwrap();
        Self {
            root,
            bin: env!("CARGO_BIN_EXE_wocli").into(),
        }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(&self.bin);
        c.env("WOCLI_HOME", self.root.path().join("data"))
            .env("WOCLI_AUDIO_OUTPUT", "null")
            .env("PATH", "/no-python-node-or-mpv");
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            c.env_remove(key);
        }
        c
    }
    fn cli(&self, args: &[&str]) -> Value {
        let p = self.command().args(args).arg("--json").output().unwrap();
        assert!(p.status.success(), "{}", String::from_utf8_lossy(&p.stderr));
        serde_json::from_slice(&p.stdout).unwrap()
    }
    fn store(&self) -> Store {
        Store::open(&Paths {
            home: self.root.path().join("data"),
            runtime: self.root.path().join("run"),
        })
        .unwrap()
    }
    fn song(&self) -> Track {
        let path = self.root.path().join("MCP test 测试.wav");
        fs::write(&path, wav()).unwrap();
        let track = Track::local(&path).unwrap();
        let store = self.store();
        store.remember(std::slice::from_ref(&track)).unwrap();
        store
            .save_lyric(&track.id, "[00:00]MCP local fixture")
            .unwrap();
        track
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = self
            .command()
            .args(["daemon", "stop"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
fn wav() -> Vec<u8> {
    let samples = 8000 * 30;
    let mut b = vec![];
    b.extend(b"RIFF");
    b.extend((36 + samples * 2u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((samples * 2).to_le_bytes());
    b.resize(44 + samples as usize * 2, 0);
    b
}

struct Client {
    child: Child,
    input: Option<Box<dyn Write>>,
    output: Receiver<Value>,
    next: u64,
    modern: bool,
    log: PathBuf,
}
impl Client {
    fn new(app: &App, read_only: bool) -> Self {
        Self::with_stdin(app, read_only, Stdio::piped())
    }
    fn with_stdin(app: &App, read_only: bool, stdin: Stdio) -> Self {
        let log = app
            .root
            .path()
            .join(format!("mcp-{}.stderr", rand::random::<u32>()));
        let mut cmd = app.command();
        cmd.args(["mcp", "serve"]);
        if read_only {
            cmd.arg("--read-only");
        }
        let mut child = cmd
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let input = child.stdin.take().map(|s| Box::new(s) as Box<dyn Write>);
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                let value = serde_json::from_str(&line)
                    .unwrap_or_else(|_| panic!("non-MCP stdout: {line}"));
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output: rx,
            next: 1,
            modern: false,
            log,
        }
    }
    fn send(&mut self, value: Value) {
        let input = self.input.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &value).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
    }
    fn start(&mut self, version: &str) {
        if version == "2026-07-28" {
            self.modern = true;
            let result = self.rpc("server/discover", json!({}));
            assert!(
                result["result"]["supportedVersions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == version),
                "{result}"
            );
        } else {
            let result=self.rpc("initialize",json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"wocli-test","version":"1"}}));
            assert_eq!(result["result"]["protocolVersion"], version, "{result}");
            self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        }
    }
    fn submit(&mut self, method: &str, mut params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        if self.modern {
            params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"wocli-test","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}});
        }
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        id
    }
    fn receive(&self, id: u64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let value = self.output.recv_timeout(remaining).unwrap_or_else(|e| {
                panic!(
                    "MCP request {id}: {e}; {}",
                    fs::read_to_string(&self.log).unwrap_or_default()
                )
            });
            if value["id"] == id {
                return value;
            }
        }
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.submit(method, params);
        self.receive(id)
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        let r = self.rpc("tools/call", json!({"name":name,"arguments":args}));
        assert!(
            r["error"].is_null() && r["result"]["isError"] != true,
            "{name}: {r}"
        );
        r["result"]["structuredContent"].clone()
    }
    fn phase(&mut self, phase: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let s = self.call("get_playback_status", json!({}));
            if s["phase"] == phase {
                return s;
            }
            assert!(Instant::now() < deadline, "{s}");
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn finish(mut self) {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "{}",
                    fs::read_to_string(&self.log).unwrap()
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "MCP did not exit after stdin EOF"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn stdio_legacy_and_modern_discovery_resources_and_config_work() {
    let app = App::new();
    for version in [
        "2024-11-05",
        "2025-03-26",
        "2025-06-18",
        "2025-11-25",
        "2026-07-28",
    ] {
        let mut c = Client::new(&app, false);
        c.start(version);
        let list = c.rpc("tools/list", json!({}));
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 17);
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
        if version == "2026-07-28" {
            assert_eq!(list["result"]["resultType"], "complete");
            // An oversized request is discarded without poisoning the next message.
            c.submit(
                "tools/call",
                json!({"name":"search_music","arguments":{"keyword":"x".repeat(70000)}}),
            );
            assert_eq!(c.call("get_playback_status", json!({}))["running"], false);
        } else {
            assert!(list["result"]["resultType"].is_null());
        }
        let resources = c.rpc("resources/list", json!({}));
        assert_eq!(
            resources["result"]["resources"].as_array().unwrap().len(),
            5
        );
        let resource = c.rpc("resources/read", json!({"uri":"wocli://status"}));
        let content: Value =
            serde_json::from_str(resource["result"]["contents"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(content["running"], false);
        assert_eq!(
            c.rpc("resources/read", json!({"uri":"file:///etc/passwd"}))["error"]["code"],
            if version == "2026-07-28" {
                -32602
            } else {
                -32002
            }
        );
        assert_eq!(c.rpc("unknown/method", json!({}))["error"]["code"], -32601);
        c.finish();
    }
    let config = app.cli(&["mcp", "config", "--read-only", "--direct"]);
    let server = &config["mcpServers"]["wocli-music"];
    assert!(PathBuf::from(server["command"].as_str().unwrap()).is_absolute());
    let args = server["args"].as_array().unwrap();
    assert!(args.iter().any(|a| a == "--read-only"));
    assert!(args.iter().any(|a| a == "--direct"));
    assert_eq!(app.cli(&["doctor"])["service_running"], false);
}

#[test]
fn playback_library_config_and_queue_share_the_real_daemon() {
    let app = App::new();
    let track = app.song();
    let mut c = Client::new(&app, false);
    c.start("2025-11-25");
    c.call("play_music", json!({"path":track.path}));
    let s = c.phase("playing");
    assert_eq!(s["track"]["id"], track.id);
    assert_eq!(s["queue_length"], 1);
    assert!(s.get("lyrics").is_none());
    c.call("control_playback", json!({"action":"pause"}));
    assert_eq!(c.phase("paused")["phase"], "paused");
    for _ in 0..2 {
        assert_eq!(
            c.call("set_favorite", json!({"track_id":track.id,"liked":true}))["liked"],
            true
        );
    }
    assert_eq!(
        c.call("get_library", json!({"kind":"favorites"}))["total"],
        1
    );
    assert!(c.call("get_lyrics", json!({"track_id":track.id}))["lyrics"]
        .as_str()
        .unwrap()
        .contains("MCP local fixture"));
    c.call(
        "manage_playlist",
        json!({"action":"create","name":"MCP 歌单"}),
    );
    c.call(
        "manage_playlist",
        json!({"action":"add","name":"MCP 歌单","track_id":track.id}),
    );
    assert_eq!(
        c.call("get_library", json!({"kind":"playlist","name":"MCP 歌单"}))["total"],
        1
    );
    c.call(
        "manage_queue",
        json!({"action":"add","track_ids":[track.id],"next":true}),
    );
    assert_eq!(c.call("get_queue", json!({"limit":1}))["next_offset"], 1);
    let bad=c.rpc("tools/call",json!({"name":"manage_queue","arguments":{"action":"remove","index":2,"track_id":"local:wrong"}}));
    assert_eq!(bad["result"]["isError"], true);
    assert_eq!(c.call("get_queue", json!({}))["total"], 2);
    c.call(
        "manage_queue",
        json!({"action":"remove","index":2,"track_id":track.id}),
    );
    assert_eq!(c.call("get_queue", json!({}))["total"], 1);
    c.call("configure_player", json!({"key":"volume","value":43}));
    assert_eq!(c.call("get_player_config", json!({}))["volume"], 43);
    let bad = c.rpc(
        "tools/call",
        json!({"name":"control_playback","arguments":{"action":"volume","value":101}}),
    );
    assert_eq!(bad["result"]["isError"], true);
    assert_eq!(c.call("get_player_config", json!({}))["volume"], 43);
    c.call(
        "manage_music_sources",
        json!({"action":"prefer","source_id":"kuwo-mobile"}),
    );
    assert_eq!(
        c.call("list_music_sources", json!({}))["sources"][0]["id"],
        "kuwo-mobile"
    );
    c.finish();
    assert_eq!(app.cli(&["status"])["phase"], "paused");
    let mut c = Client::new(&app, false);
    c.start("2026-07-28");
    assert_eq!(c.call("get_playback_status", json!({}))["phase"], "paused");
    c.call("control_playback", json!({"action":"stop"}));
    c.finish();
}

#[test]
fn read_only_and_typed_validation_prevent_hidden_mutations_and_url_leaks() {
    let app = App::new();
    let track = app.song();
    let mut sources = wocli_music::sources::Sources::default();
    sources.sources.push(wocli_music::sources::Source {
        id: "secret_api".into(),
        name: "Private API".into(),
        enabled: false,
        builtin: false,
        qualities: vec!["128k".into()],
        kind: wocli_music::sources::SourceKind::Http {
            url: "https://example.test/api?token=private-secret".into(),
            response_path: Some("/url".into()),
        },
    });
    fs::write(
        app.root.path().join("data/sources.json"),
        serde_json::to_vec(&sources).unwrap(),
    )
    .unwrap();
    let mut c = Client::new(&app, true);
    c.start("2025-11-25");
    let tools = c.rpc("tools/list", json!({}));
    let tools = tools["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 8);
    assert!(tools
        .iter()
        .all(|t| t["annotations"]["readOnlyHint"] == true));
    let hidden = c.rpc(
        "tools/call",
        json!({"name":"set_favorite","arguments":{"track_id":track.id,"liked":true}}),
    );
    assert!(!hidden["error"].is_null() || hidden["result"]["isError"] == true);
    assert_eq!(
        c.call("get_library", json!({"kind":"favorites"}))["total"],
        0
    );
    let bad = c.rpc(
        "tools/call",
        json!({"name":"get_playback_status","arguments":{"op":"shutdown"}}),
    );
    assert_eq!(bad["result"]["isError"], true);
    let bad = c.rpc(
        "tools/call",
        json!({"name":"get_library","arguments":{"kind":"favorites","limit":0}}),
    );
    assert_eq!(bad["result"]["isError"], true);
    let sources = c.call("list_music_sources", json!({}));
    assert!(!sources.to_string().contains("private-secret"));
    assert!(!sources.to_string().contains("example.test"));
    assert_eq!(c.call("get_playback_status", json!({}))["running"], false);
    c.finish();
    assert_eq!(app.cli(&["doctor"])["service_running"], false);
}

#[test]
fn eof_before_handshake_and_invalid_cli_exit_cleanly() {
    let app = App::new();
    let output = app
        .command()
        .arg("mcp")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let output = app.command().args(["mcp", "http"]).output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn connection_prompt_contains_local_config_and_respects_options() {
    let app = App::new();
    let home = app.root.path().join("自选 曲库");
    for read_only in [false, true] {
        let mut args = vec![
            "mcp",
            "prompt",
            "--direct",
            "--home",
            home.to_str().unwrap(),
        ];
        if read_only {
            args.push("--read-only");
        }
        let output = app.command().args(&args).output().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let plain = String::from_utf8(output.stdout).unwrap();
        let value = app.cli(&args);
        assert_eq!(plain.trim_end(), value["prompt"].as_str().unwrap());
        let config_text = plain
            .split("```json\n")
            .nth(1)
            .unwrap()
            .split("\n```")
            .next()
            .unwrap();
        let embedded: Value = serde_json::from_str(config_text).unwrap();
        args[1] = "config";
        assert_eq!(embedded, app.cli(&args));
        assert_eq!(embedded, value["config"]);
        assert_eq!(
            embedded["mcpServers"]["wocli-music"]["args"][3],
            fs::canonicalize(&home).unwrap().to_str().unwrap()
        );
        assert!(plain.contains("get_playback_status"));
        assert!(plain.contains("不要自动播放或下载"));
        assert!(plain.contains("不要声称已经连接"));
        assert_eq!(plain.contains("这是只读连接"), read_only);
    }
    assert_eq!(app.cli(&["doctor"])["service_running"], false);
}

#[cfg(unix)]
#[test]
fn terminal_shows_copyable_guide_keeps_rpc_clean_and_handles_ctrl_c() {
    use std::os::fd::FromRawFd;
    let app = App::new();
    for initialize in [false, true] {
        let (mut master, mut slave) = (-1, -1);
        // A real PTY makes stdin.is_terminal() true, including on CI without a console.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let master = unsafe { fs::File::from_raw_fd(master) };
        let slave = unsafe { fs::File::from_raw_fd(slave) };
        let mut c = Client::with_stdin(&app, true, Stdio::from(slave));
        c.input = Some(Box::new(master));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let text = fs::read_to_string(&c.log).unwrap();
            if text.contains("下次可用 wocli mcp prompt") {
                assert!(text.contains("MCP 服务启动成功"));
                assert!(text.contains("复制下面的提示词给 AI"));
                assert!(text.contains("当前尚未连接 AI"));
                assert!(text.contains("这是只读连接"));
                assert!(text.contains("\"--read-only\""));
                break;
            }
            assert!(Instant::now() < deadline, "no startup guide: {text}");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(
            c.output.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        if initialize {
            c.start("2025-11-25");
            assert_eq!(
                c.rpc("tools/list", json!({}))["result"]["tools"]
                    .as_array()
                    .unwrap()
                    .len(),
                8
            );
        }
        assert_eq!(
            unsafe { libc::kill(c.child.id() as libc::pid_t, libc::SIGINT) },
            0
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = c.child.try_wait().unwrap() {
                assert!(status.success(), "{}", fs::read_to_string(&c.log).unwrap());
                break;
            }
            assert!(Instant::now() < deadline, "Ctrl+C did not stop MCP");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

struct AudioServer {
    url: String,
    resume: std::sync::Arc<std::sync::atomic::AtomicBool>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl AudioServer {
    fn new() -> Self {
        use std::{
            net::TcpListener,
            sync::{
                atomic::{AtomicBool, Ordering},
                Arc,
            },
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/song", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let quit = stop.clone();
        let resume = Arc::new(AtomicBool::new(false));
        let ready = resume.clone();
        let worker = thread::spawn(move || {
            let body = wav();
            while !quit.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or_default() == 0 {
                    continue;
                }
                let mut range = None;
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or_default() == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        let (start, end) = value.trim().split_once('-').unwrap();
                        range = Some((
                            start.parse::<usize>().unwrap(),
                            end.parse::<usize>()
                                .unwrap_or(body.len() - 1)
                                .min(body.len() - 1),
                        ));
                    }
                }
                if let Some((start, end)) = range {
                    let _=write!(socket,"HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nConnection: close\r\n\r\n",end-start+1,body.len());
                    let _ = socket.write_all(&body[start..=end]);
                } else {
                    let _ = write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(&body[..300000]);
                    while !ready.load(Ordering::Relaxed) && !quit.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(5));
                    }
                    if !quit.load(Ordering::Relaxed) {
                        let _ = socket.write_all(&body[300000..]);
                    }
                }
            }
        });
        Self {
            url,
            resume,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for AudioServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.worker.take().unwrap().join();
    }
}

#[test]
fn cancelling_a_waiting_seek_keeps_stdio_responsive_and_downloads_are_background_jobs() {
    let app = App::new();
    let server = AudioServer::new();
    let source = wocli_music::sources::Source {
        id: "test_audio".into(),
        name: "Test audio".into(),
        enabled: true,
        builtin: false,
        qualities: vec!["128k".into()],
        kind: wocli_music::sources::SourceKind::Http {
            url: server.url.clone(),
            response_path: None,
        },
    };
    fs::write(
        app.root.path().join("data/sources.json"),
        serde_json::to_vec(&wocli_music::sources::Sources {
            sources: vec![source],
        })
        .unwrap(),
    )
    .unwrap();
    let track = Track {
        id: "kuwo:123".into(),
        title: "MCP test stream".into(),
        duration: 30.,
        ..Default::default()
    };
    let store = app.store();
    store.remember(std::slice::from_ref(&track)).unwrap();
    store.save_lyric(&track.id, "[00:00]test").unwrap();
    drop(store);
    let mut c = Client::new(&app, false);
    c.start("2025-11-25");
    c.call("play_music", json!({"track_id":track.id,"quality":"128k"}));
    c.phase("playing");
    let pending = c.submit(
        "tools/call",
        json!({"name":"control_playback","arguments":{"action":"seek","seconds":25}}),
    );
    thread::sleep(Duration::from_millis(60));
    c.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":pending,"reason":"test cancellation"}}));
    let started = Instant::now();
    assert_eq!(c.call("get_playback_status", json!({}))["running"], true);
    assert!(started.elapsed() < Duration::from_secs(2));
    c.call("control_playback", json!({"action":"stop"}));
    server
        .resume
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let directory = app.root.path().join("downloads");
    let started = Instant::now();
    let result = c.call(
        "download_music",
        json!({"track_id":track.id,"quality":"128k","directory":directory}),
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    let job_id = result["job_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let jobs = c.call("get_downloads", json!({}));
        let job = jobs["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["id"] == job_id)
            .unwrap();
        assert_ne!(job["phase"], "error", "{job}");
        if job["phase"] == "done" {
            assert_eq!(fs::read(job["path"].as_str().unwrap()).unwrap(), wav());
            break;
        }
        assert!(Instant::now() < deadline, "{job}");
        thread::sleep(Duration::from_millis(20));
    }
    c.finish();
}
