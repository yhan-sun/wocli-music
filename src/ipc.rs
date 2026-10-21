use crate::{
    model::{Error, Result},
    store::Paths,
};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::net::UnixStream;

#[cfg(windows)]
use std::net::TcpStream;

#[derive(Debug)]
pub enum Stream {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Tcp(TcpStream),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.read(buf),
            #[cfg(windows)]
            Stream::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.write(buf),
            #[cfg(windows)]
            Stream::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.flush(),
            #[cfg(windows)]
            Stream::Tcp(s) => s.flush(),
        }
    }
}

impl Stream {
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.set_read_timeout(timeout),
            #[cfg(windows)]
            Stream::Tcp(s) => s.set_read_timeout(timeout),
        }
    }
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.set_write_timeout(timeout),
            #[cfg(windows)]
            Stream::Tcp(s) => s.set_write_timeout(timeout),
        }
    }
    pub fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.try_clone().map(Stream::Unix),
            #[cfg(windows)]
            Stream::Tcp(s) => s.try_clone().map(Stream::Tcp),
        }
    }
}

#[cfg(unix)]
pub fn connect(paths: &Paths) -> Result<Stream> {
    Ok(Stream::Unix(UnixStream::connect(paths.socket())?))
}

#[cfg(windows)]
pub fn connect(paths: &Paths) -> Result<Stream> {
    use std::fs;
    let content = fs::read_to_string(paths.socket())?;
    let mut lines = content.lines();
    let port: u16 = lines
        .next()
        .ok_or_else(|| Error::new("后台端口未知"))?
        .trim()
        .parse()
        .map_err(|_| Error::new("无效后台端口"))?;
    let token = lines
        .next()
        .ok_or_else(|| Error::new("后台授权信息缺失"))?
        .trim();
    let mut tcp = TcpStream::connect(("127.0.0.1", port))?;
    let _ = tcp.set_nodelay(true);
    tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    tcp.write_all(format!("{token}\n").as_bytes())?;
    Ok(Stream::Tcp(tcp))
}

pub fn request(paths: &Paths, value: Value, autostart: bool) -> Result<Option<Value>> {
    let stream = match connect(paths) {
        Ok(stream) => stream,
        Err(_) => {
            if !autostart {
                return Ok(None);
            }
            start(paths)?;
            connect(paths)?
        }
    };
    let shutdown = value["op"] == "shutdown";
    let response = exchange(stream, value)?;
    if shutdown {
        use fs2::FileExt;
        let lock_file = paths.runtime.join("service.lock");
        if lock_file.exists() {
            if let Ok(lock) = OpenOptions::new().write(true).open(&lock_file) {
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                loop {
                    if lock.try_lock_exclusive().is_ok() {
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(Error::new("后台仍在清理下载任务，请稍后重试"));
                    }
                    thread::sleep(Duration::from_millis(30));
                }
            }
        }
    }
    Ok(Some(response))
}

pub fn exchange(mut stream: Stream, value: Value) -> Result<Value> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    serde_json::to_writer(&mut stream, &value)?;
    stream.write_all(b"\n")?;
    let mut response = String::new();
    BufReader::new(stream)
        .take(2 * 1024 * 1024)
        .read_line(&mut response)?;
    let result: Value =
        serde_json::from_str(&response).map_err(|_| Error::new("后台服务响应无效"))?;
    if result["ok"] == true {
        Ok(result["data"].clone())
    } else {
        Err(Error {
            message: result["error"]
                .as_str()
                .unwrap_or("后台服务请求失败")
                .into(),
            code: result["code"].as_str().unwrap_or("error").into(),
            details: serde_json::from_value(result["details"].clone()).unwrap_or_default(),
        })
    }
}

pub fn start(paths: &Paths) -> Result<()> {
    #[cfg(unix)]
    {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(paths.home.join("service-rust.log"))?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("__daemon")
            .arg("--home")
            .arg(&paths.home)
            .env("WOCLI_HOME", &paths.home)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = command.spawn()?;
        for _ in 0..600 {
            if connect(paths).is_ok() {
                return Ok(());
            }
            if child.try_wait()?.is_some_and(|status| !status.success()) {
                return Err(Error::new(format!(
                    "后台服务启动失败：{}",
                    paths.home.join("service-rust.log").display()
                )));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()?;
        let cmd_line = format!(
            "\"{}\" __daemon --home \"{}\"",
            exe.to_string_lossy(),
            paths.home.to_string_lossy()
        );
        let ps_cmd = format!(
            "Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{{CommandLine='{}'}}",
            cmd_line.replace('\'', "''")
        );
        let spawned = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &ps_cmd,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !spawned {
            let mut cmd = Command::new("cmd");
            cmd.args(["/c", "start", "/b", ""])
                .arg(&exe)
                .arg("__daemon")
                .arg("--home")
                .arg(&paths.home)
                .env("WOCLI_HOME", &paths.home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let _ = cmd.spawn();
        }
        for _ in 0..600 {
            if connect(paths).is_ok() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    Err(Error::new("后台服务启动超时"))
}

pub fn control(paths: &Paths, op: &str) -> Result<Value> {
    request(paths, json!({"op":op}), true)?.ok_or_else(|| Error::new("后台服务未运行"))
}
