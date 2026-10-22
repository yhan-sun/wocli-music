use crate::{
    model::{Error, Result},
    store::Paths,
};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Read, Write},
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::{
    os::unix::net::UnixStream,
    process::{Command, Stdio},
};

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
static ENDPOINT_CACHE: std::sync::Mutex<Option<(u16, String)>> = std::sync::Mutex::new(None);

#[cfg(windows)]
pub fn connect(paths: &Paths) -> Result<Stream> {
    use std::fs;

    if let Ok(guard) = ENDPOINT_CACHE.lock() {
        if let Some((port, token)) = guard.as_ref() {
            if let Ok(mut tcp) = TcpStream::connect(("127.0.0.1", *port)) {
                let _ = tcp.set_nodelay(true);
                let _ = tcp.set_linger(Some(Duration::ZERO));
                let _ = tcp.set_write_timeout(Some(Duration::from_secs(5)));
                let _ = tcp.set_read_timeout(Some(Duration::from_secs(5)));
                if tcp.write_all(format!("{token}\n").as_bytes()).is_ok() {
                    return Ok(Stream::Tcp(tcp));
                }
            }
        }
    }

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
    let _ = tcp.set_linger(Some(Duration::ZERO));
    tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    tcp.write_all(format!("{token}\n").as_bytes())?;

    if let Ok(mut guard) = ENDPOINT_CACHE.lock() {
        *guard = Some((port, token.to_string()));
    }

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
        use std::os::windows::io::AsRawHandle;
        let process = spawn_detached(paths, &["__daemon"])?;
        for _ in 0..600 {
            if connect(paths).is_ok() {
                return Ok(());
            }
            let mut code = 0u32;
            let exited = unsafe {
                windows_sys::Win32::System::Threading::GetExitCodeProcess(
                    process.as_raw_handle() as isize,
                    &mut code,
                ) != 0
                    && code != 259
            };
            if exited {
                return Err(Error::new(format!("后台进程启动失败（退出码 {code}）")));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    Err(Error::new("后台服务启动超时"))
}

/// Native detached startup preserves WOCLI_* settings and never inherits the
/// CLI's pipe handles. A shell/WMI launcher can leak handles or lose this environment.
#[cfg(windows)]
pub fn spawn_detached(
    paths: &Paths,
    arguments: &[&str],
) -> Result<std::os::windows::io::OwnedHandle> {
    use std::{
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            CreateProcessW, CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
            PROCESS_INFORMATION, STARTUPINFOW,
        },
    };
    let executable = std::env::current_exe()?;
    let mut application: Vec<u16> = executable.as_os_str().encode_wide().collect();
    application.push(0);
    let mut command = quote_windows_argument(executable.as_os_str());
    for argument in arguments {
        command.push(b' ' as u16);
        command.extend(quote_windows_argument(std::ffi::OsStr::new(argument)));
    }
    command.push(b' ' as u16);
    command.extend(quote_windows_argument(std::ffi::OsStr::new("--home")));
    command.push(b' ' as u16);
    command.extend(quote_windows_argument(paths.home.as_os_str()));
    command.push(0);
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // Break away from a parent job when allowed; restricted hosts such as CI
    // still get a detached child in the same job, with no inherited handles.
    for flags in [
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB,
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
    ] {
        let mut command_line = command.clone();
        let created = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                flags,
                ptr::null(),
                ptr::null(),
                &startup,
                &mut process,
            )
        };
        if created != 0 {
            unsafe {
                CloseHandle(process.hThread);
            }
            return Ok(unsafe {
                std::os::windows::io::OwnedHandle::from_raw_handle(
                    process.hProcess as *mut std::ffi::c_void,
                )
            });
        }
    }
    Err(Error::new(format!(
        "无法启动后台进程: {}",
        std::io::Error::last_os_error()
    )))
}
#[cfg(windows)]
fn quote_windows_argument(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for character in value.encode_wide() {
        if character == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        let escaped = if character == b'"' as u16 {
            slashes * 2 + 1
        } else {
            slashes
        };
        result.extend(std::iter::repeat_n(b'\\' as u16, escaped));
        slashes = 0;
        result.push(character);
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    result
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    #[test]
    fn command_arguments_preserve_spaces_quotes_unicode_and_trailing_slashes() {
        for (argument, expected) in [
            ("C:\\Music Folder\\", "\"C:\\Music Folder\\\\\""),
            ("a\"b", "\"a\\\"b\""),
            ("音乐", "\"音乐\""),
        ] {
            assert_eq!(
                String::from_utf16(&quote_windows_argument(std::ffi::OsStr::new(argument)))
                    .unwrap(),
                expected
            );
        }
    }
}

pub fn control(paths: &Paths, op: &str) -> Result<Value> {
    request(paths, json!({"op":op}), true)?.ok_or_else(|| Error::new("后台服务未运行"))
}
