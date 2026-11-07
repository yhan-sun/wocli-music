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

// A supported 10,000-track playlist can exceed the old 2 MiB frame. Apply the
// same bound to requests and responses, and report overflow before JSON parsing.
const MAX_MESSAGE_BYTES: u64 = 64 * 1024 * 1024;

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
    let shutdown = value["op"] == "shutdown";
    let stream = match connect(paths) {
        Ok(stream) => stream,
        Err(_) => {
            if !autostart {
                if shutdown {
                    // The endpoint disappears before worker cleanup releases
                    // the lock. A repeated stop must still wait for completion.
                    wait_for_shutdown(paths, Duration::from_secs(30))?;
                }
                return Ok(None);
            }
            start(paths)?;
            connect(paths)?
        }
    };
    let response = match exchange(stream, value) {
        // Older daemons can exit before the IPC handler flushes its reply.
        // A closed connection alone is not success: confirm cleanup via lock.
        Err(error) if shutdown && error.code == "ipc_closed" => {
            wait_for_shutdown(paths, Duration::from_secs(30))?;
            return Ok(None);
        }
        result => result?,
    };
    if shutdown {
        wait_for_shutdown(paths, Duration::from_secs(30))?;
    }
    Ok(Some(response))
}

fn wait_for_shutdown(paths: &Paths, timeout: Duration) -> Result<()> {
    use fs2::FileExt;
    let lock = match OpenOptions::new()
        .write(true)
        .open(paths.runtime.join("service.lock"))
    {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == fs2::lock_contended_error().kind() => {}
            Err(error) => return Err(error.into()),
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new("后台仍在清理任务，请稍后重试"));
        }
        thread::sleep(Duration::from_millis(30));
    }
}

pub fn exchange(mut stream: Stream, value: Value) -> Result<Value> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write_message(&mut stream, &value)?;
    let result = read_message(stream)?;
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

pub(crate) fn read_message(stream: impl Read) -> Result<Value> {
    read_message_with_limit(stream, MAX_MESSAGE_BYTES)
}

fn transport_error(error: std::io::Error) -> Error {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::BrokenPipe => {
            Error::coded("ipc_closed", format!("后台连接已断开：{error}"))
        }
        _ => error.into(),
    }
}

pub(crate) fn write_message(stream: impl Write, message: &Value) -> Result<()> {
    write_message_with_limit(stream, message, MAX_MESSAGE_BYTES)
}

fn write_message_with_limit(mut stream: impl Write, message: &Value, limit: u64) -> Result<()> {
    let mut bytes = serde_json::to_vec(message)?;
    if bytes.len() as u64 >= limit {
        return Err(message_too_large(limit));
    }
    bytes.push(b'\n');
    stream.write_all(&bytes).map_err(transport_error)
}

fn message_too_large(limit: u64) -> Error {
    Error::coded(
        "ipc_message_too_large",
        format!("IPC 消息超过长度上限（{limit} 字节）"),
    )
}

fn read_message_with_limit(stream: impl Read, limit: u64) -> Result<Value> {
    let mut message = Vec::new();
    let received = BufReader::new(stream)
        .take(limit + 1)
        .read_until(b'\n', &mut message)
        .map_err(transport_error)?;
    if received == 0 {
        return Err(Error::coded("ipc_closed", "后台连接已断开，未收到响应"));
    }
    if received as u64 > limit {
        return Err(message_too_large(limit));
    }
    serde_json::from_slice(&message).map_err(|_| {
        if !message.ends_with(b"\n") {
            // Shutdown may end the process while its IPC thread is flushing a
            // large reply. Let request() verify daemon cleanup via its lock.
            Error::coded("ipc_closed", "后台连接在消息传输完成前断开")
        } else {
            Error::new("后台服务响应无效")
        }
    })
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
    spawn_windows_process(paths, arguments, false)
}

/// Open the same library in a fresh native console without `cmd /c start`
/// interpreting executable or home paths as shell syntax.
#[cfg(windows)]
pub fn spawn_console(paths: &Paths) -> Result<std::os::windows::io::OwnedHandle> {
    spawn_windows_process(paths, &["tui"], true)
}

#[cfg(windows)]
fn spawn_windows_process(
    paths: &Paths,
    arguments: &[&str],
    console: bool,
) -> Result<std::os::windows::io::OwnedHandle> {
    use std::{
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            CreateProcessW, CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_CONSOLE,
            CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, PROCESS_INFORMATION, STARTUPINFOW,
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
    let base_flags = if console {
        CREATE_NEW_CONSOLE
    } else {
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    };
    for flags in [base_flags | CREATE_BREAKAWAY_FROM_JOB, base_flags] {
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

#[cfg(test)]
mod framing_tests {
    use super::*;

    #[test]
    fn frame_limit_accepts_exact_fit_and_reports_overflow_instead_of_invalid_json() {
        let message = b"{\"queue\":[1,2,3]}\n";
        assert_eq!(
            read_message_with_limit(&message[..], message.len() as u64).unwrap(),
            json!({"queue":[1,2,3]})
        );
        let error = read_message_with_limit(&message[..], message.len() as u64 - 1).unwrap_err();
        assert_eq!(error.code, "ipc_message_too_large");
        assert_eq!(read_message(&b""[..]).unwrap_err().code, "ipc_closed");
        assert_eq!(
            read_message(&b"{\"queue\":["[..]).unwrap_err().code,
            "ipc_closed"
        );
        assert_ne!(
            read_message(&b"{invalid}\n"[..]).unwrap_err().code,
            "ipc_closed"
        );
        let mut output = Vec::new();
        let value = json!({"queue":[1,2,3]});
        assert_eq!(
            write_message_with_limit(&mut output, &value, message.len() as u64 - 1)
                .unwrap_err()
                .code,
            "ipc_message_too_large"
        );
        assert!(output.is_empty());
        write_message_with_limit(&mut output, &value, message.len() as u64).unwrap();
        assert_eq!(output, message);
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use fs2::FileExt;
    use std::sync::mpsc;

    #[test]
    fn repeated_shutdown_waits_for_cleanup_without_an_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: dir.path().into(),
            runtime: dir.path().into(),
        };
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(paths.runtime.join("service.lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        let (tx, rx) = mpsc::channel();
        let client = thread::spawn(move || {
            tx.send(request(&paths, json!({"op":"shutdown"}), false))
                .unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(lock);
        assert!(rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .is_none());
        client.join().unwrap();
    }

    #[test]
    fn shutdown_timeout_is_not_reported_as_success() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: dir.path().into(),
            runtime: dir.path().into(),
        };
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(paths.runtime.join("service.lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        assert!(wait_for_shutdown(&paths, Duration::from_millis(20)).is_err());
        drop(lock);
        wait_for_shutdown(&paths, Duration::ZERO).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_confirms_exit_when_daemon_disconnects_before_reply() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: dir.path().into(),
            runtime: dir.path().into(),
        };
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(paths.runtime.join("service.lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        let listener = std::os::unix::net::UnixListener::bind(paths.socket()).unwrap();
        let (release, cleanup) = mpsc::channel();
        let daemon = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&line).unwrap()["op"],
                "shutdown"
            );
            drop(reader);
            cleanup.recv().unwrap();
            drop(lock);
        });
        let (tx, rx) = mpsc::channel();
        let client = thread::spawn(move || {
            tx.send(request(&paths, json!({"op":"shutdown"}), false))
                .unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        release.send(()).unwrap();
        assert!(rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .is_none());
        client.join().unwrap();
        daemon.join().unwrap();
    }
}
