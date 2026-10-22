use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    requests: Arc<Mutex<Vec<String>>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let base = url.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let quit = stop.clone();
        let requests = Arc::new(Mutex::new(vec![]));
        let log = requests.clone();
        let worker = thread::spawn(move || {
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
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                log.lock().unwrap().push(path.clone());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if line.to_lowercase().starts_with("content-length:") {
                        length = line
                            .split(':')
                            .nth(1)
                            .unwrap()
                            .trim()
                            .parse::<usize>()
                            .unwrap();
                    }
                }
                if length > 0 {
                    let mut body = vec![0; length.min(1024 * 1024)];
                    std::io::Read::read_exact(&mut reader, &mut body).unwrap();
                }
                let (status, body, content) = if path.starts_with("/api") {
                    (
                        "200 OK",
                        serde_json::to_vec(&json!({"data":{"url":format!("{base}/good")}}))
                            .unwrap(),
                        "application/json",
                    )
                } else if path.starts_with("/bad") {
                    (
                        "200 OK",
                        b"RIFFnot really audio but passes magic header probe".to_vec(),
                        "audio/wav",
                    )
                } else if path.starts_with("/good") {
                    ("200 OK", wav(), "audio/wav")
                } else {
                    ("503 Unavailable", b"unavailable".to_vec(), "text/plain")
                };
                let _=write!(socket,"HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content}\r\nConnection: close\r\n\r\n",body.len());
                let _ = socket.write_all(&body);
            }
        });
        Self {
            url,
            stop,
            thread: Some(worker),
            requests,
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.thread.take().unwrap().join();
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
struct App {
    root: tempfile::TempDir,
    bin: PathBuf,
}
impl App {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
            bin: env!("CARGO_BIN_EXE_wocli").into(),
        }
    }
    fn output(&self, args: &[&str]) -> std::process::Output {
        Command::new(&self.bin)
            .args(args)
            .env("WOCLI_HOME", self.root.path().join("data"))
            .env("WOCLI_AUDIO_OUTPUT", "null")
            .env("PATH", "/no-node-or-python")
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env_remove("ALL_PROXY")
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("all_proxy")
            .output()
            .unwrap()
    }
    fn cli(&self, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        let out = self.output(&args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn add(&self, name: &str, url: &str, response_path: Option<&str>) -> String {
        let path = self.root.path().join(format!("{name}.json"));
        let mut value = json!({"name":name,"url":url,"qualities":["128k"]});
        if let Some(p) = response_path {
            value["response_path"] = json!(p);
        }
        fs::write(&path, value.to_string()).unwrap();
        self.cli(&["source", "add", path.to_str().unwrap()])["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn disable_builtins(&self) {
        self.cli(&["source", "disable", "kuwo"]);
        self.cli(&["source", "disable", "kuwo-mobile"]);
    }
    fn remember(&self) {
        let home = self.root.path().join("data");
        let paths = wocli_music::store::Paths {
            home: home.clone(),
            runtime: self.root.path().join("run"),
        };
        let store = wocli_music::store::Store::open(&paths).unwrap();
        let track = wocli_music::model::Track {
            id: "kuwo:123".into(),
            title: "测试换源".into(),
            duration: 30.,
            ..Default::default()
        };
        store.remember(&[track]).unwrap();
        store.save_lyric("kuwo:123", "[00:00]test").unwrap();
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.output(&["daemon", "stop"]);
    }
}

#[test]
fn help_aliases_and_topic_help_do_not_require_a_service() {
    let app = App::new();
    for topic in wocli_music::help::COMMANDS {
        let out = app.cli(&["help", topic]);
        assert!(!out["text"].as_str().unwrap().is_empty());
    }
    assert_eq!(
        app.cli(&["thelp", "source"])["text"],
        app.cli(&["source", "--help"])["text"]
    );
    assert!(app.cli(&["--help"])["text"]
        .as_str()
        .unwrap()
        .contains("source"));
    assert!(!app.root.path().join("data").exists());
    assert!(!app.output(&["help", "missing"]).status.success());
}
#[test]
fn source_management_persists_order_and_validates_imports() {
    let app = App::new();
    let server = Server::new();
    let id = app.add(
        "接口模板",
        &format!("{}/api?id={{id}}&q={{quality}}", server.url),
        Some("/data/url"),
    );
    app.cli(&["source", "prefer", &id]);
    assert_eq!(app.cli(&["source", "list"])["sources"][0]["id"], id);
    assert_eq!(
        app.cli(&["source", "test", &id, "--id", "123"])["status"],
        "ok"
    );
    app.cli(&["source", "disable", &id]);
    assert_eq!(app.cli(&["source", "show", &id])["enabled"], false);
    assert!(!app.output(&["source", "remove", "kuwo"]).status.success());
    app.cli(&["source", "remove", &id]);
    assert!(!app.output(&["source", "show", &id]).status.success());
    let path = app.root.path().join("bad.json");
    fs::write(&path, r#"{"name":"bad","url":"file:///etc/passwd"}"#).unwrap();
    assert!(!app
        .output(&["source", "add", path.to_str().unwrap()])
        .status
        .success());
}
#[test]
fn lx_local_import_http_callbacks_and_script_integrity() {
    let app = App::new();
    let server = Server::new();
    let script = format!(
        r#"/**
 * @name LX 测试
 */
const {{EVENT_NAMES,on,send,request}}=lx;
on(EVENT_NAMES.request,({{info}})=>new Promise((resolve,reject)=>{{
  request('{}'+'/api',{{method:'POST',form:{{id:info.musicInfo.songmid}},timeout:1000}},(err,resp,body)=>{{
    if(err)return reject(err);
    if(resp.statusCode!==200 || body!==resp.body)return reject(new Error('callback shape'));
    resolve(body.data.url);
  }});
}}));
Promise.resolve().then(()=>send(EVENT_NAMES.inited,{{sources:{{kw:{{actions:['musicUrl'],qualitys:['128k']}}}}}}));"#,
        server.url
    );
    let path = app.root.path().join("source.js");
    fs::write(&path, script).unwrap();
    let source = app.cli(&["source", "add", path.to_str().unwrap()]);
    let id = source["id"].as_str().unwrap();
    assert_eq!(source["name"], "LX 测试");
    assert_eq!(
        app.cli(&["source", "test", id, "--id", "123"])["status"],
        "ok"
    );
    let file = app
        .root
        .path()
        .join("data/sources/scripts")
        .join(format!("{}.js", source["sha256"].as_str().unwrap()));
    fs::write(file, "tampered").unwrap();
    assert!(!app.output(&["source", "test", id]).status.success());
}
#[test]
fn playback_and_download_change_sources_after_bad_audio_and_reload_live() {
    let app = App::new();
    let server = Server::new();
    app.disable_builtins();
    let bad = app.add("broken", &format!("{}/bad", server.url), None);
    let good = app.add("working", &format!("{}/good", server.url), None);
    app.remember();
    let state = app.cli(&["play", "--id", "123", "--quality", "128k"]);
    assert_eq!(state["phase"], "playing");
    assert_eq!(state["source"], good);
    assert!(!state["source_failures"].as_array().unwrap().is_empty());
    let path = app.root.path().join("downloads");
    let download = app.cli(&[
        "download",
        "--id",
        "123",
        "--quality",
        "128k",
        "--output",
        path.to_str().unwrap(),
    ]);
    assert_eq!(fs::read(download["path"].as_str().unwrap()).unwrap(), wav());
    // A running daemon reads the changed configuration on the next request.
    app.cli(&["source", "disable", &good]);
    // A failed --force download must leave the previous file intact.
    fs::write(download["path"].as_str().unwrap(), b"previous file").unwrap();
    let output = app.output(&[
        "download",
        "--id",
        "123",
        "--quality",
        "128k",
        "--force",
        "--output",
        path.to_str().unwrap(),
        "--json",
    ]);
    assert!(!output.status.success());
    assert_eq!(
        fs::read(download["path"].as_str().unwrap()).unwrap(),
        b"previous file"
    );
    assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
    let output = app.output(&["retry", "--quality", "128k", "--json"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("source_unavailable"));
    let state = app.cli(&["status"]);
    assert_eq!(state["phase"], "error");
    app.cli(&["source", "disable", &bad]);
    let start = Instant::now();
    assert!(!app.output(&["retry"]).status.success());
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(server
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|p| p.starts_with("/bad")));
}
