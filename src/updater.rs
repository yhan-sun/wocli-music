use crate::{
    ipc,
    model::{Error, Result},
    store::Paths,
    tray,
};
use fs2::FileExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};

const REPO: &str = "yhan-sun/wocli-music";
const DISTRIBUTION: &str = "https://wocli.yhan.fun";

fn distribution(value: Option<&str>) -> Result<(String, String)> {
    let base = value.unwrap_or(DISTRIBUTION).trim_end_matches('/');
    if base == "github" {
        return Ok((
            format!("https://api.github.com/repos/{REPO}/releases/latest"),
            format!("https://github.com/{REPO}"),
        ));
    }
    let parsed = ureq::get(base)
        .request_url()
        .map_err(|_| Error::new("WOCLI_DOWNLOAD_BASE 不是有效 URL"))?;
    let url = parsed.as_url();
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::new(
            "WOCLI_DOWNLOAD_BASE 必须为不含凭据、查询参数的 HTTPS 地址，或 github",
        ));
    }
    Ok((format!("{base}/api/releases/latest"), base.to_string()))
}

const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
const MAX_UNPACKED: u64 = 256 * 1024 * 1024;

#[derive(Deserialize)]
struct Asset {
    name: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}
impl Release {
    fn version(&self) -> Result<Version> {
        let version = Version::parse(self.tag_name.trim_start_matches('v'))
            .map_err(|_| Error::new("发行版本号无效"))?;
        if self.draft || self.prerelease || !version.pre.is_empty() {
            return Err(Error::new("最新发行版不是稳定版本"));
        }
        Ok(version)
    }
    fn asset(&self, base: &str, name: &str) -> Result<String> {
        if !self.assets.iter().any(|a| a.name == name) {
            return Err(Error::new(format!("发行版缺少 {name}")));
        }
        Ok(format!("{base}/releases/download/{}/{name}", self.tag_name))
    }
}
#[derive(Serialize)]
pub struct UpdateReport {
    pub status: String,
    pub current_version: String,
    pub latest_version: String,
    pub executable: PathBuf,
    pub release_url: String,
    pub warning: String,
}

fn platform(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("macos", "x86_64") => Ok("macos-x86_64"),
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("windows", "x86_64" | "aarch64") => Ok("windows-x86_64"),
        _ => Err(Error::new(format!("暂未提供 {os}/{arch} 的发行包"))),
    }
}
fn response(agent: &ureq::Agent, url: &str) -> Result<ureq::Response> {
    agent
        .get(url)
        .call()
        .map_err(|e| Error::coded("network", format!("更新请求失败: {e}")))
}
fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::new("更新响应超过大小限制"));
    }
    Ok(bytes)
}
fn download(agent: &ureq::Agent, url: &str, path: &Path) -> Result<String> {
    let mut reader = response(agent, url)?.into_reader();
    let mut file = File::create(path)?;
    let mut sha = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0; 65536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > MAX_ARCHIVE {
            return Err(Error::new("更新包超过大小限制"));
        }
        sha.update(&buffer[..n]);
        file.write_all(&buffer[..n])?;
    }
    file.sync_all()?;
    Ok(format!("{:x}", sha.finalize()))
}
fn checksum(sums: &str, name: &str) -> Result<String> {
    let mut found = None;
    for line in sums.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() == 2 && parts[1].trim_start_matches('*') == name {
            if found.is_some()
                || parts[0].len() != 64
                || !parts[0].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(Error::new("SHA256SUMS 中的校验值无效或重复"));
            }
            found = Some(parts[0].to_lowercase());
        }
    }
    found.ok_or_else(|| Error::new(format!("SHA256SUMS 缺少 {name} 的校验值")))
}
fn safe_path(path: &Path, root: &str) -> Result<()> {
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        || !path.starts_with(root)
    {
        return Err(Error::new("发行包包含无效路径"));
    }
    Ok(())
}
fn unpack(archive: &Path, directory: &Path, root: &str, zip: bool) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let mut total = 0u64;
    if zip {
        let mut archive = zip::ZipArchive::new(File::open(archive)?)
            .map_err(|e| Error::new(format!("ZIP 解压失败: {e}")))?;
        if archive.len() > 20000 {
            return Err(Error::new("发行包文件过多"));
        }
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| Error::new(e.to_string()))?;
            let path = entry
                .enclosed_name()
                .ok_or_else(|| Error::new("ZIP 包含无效路径"))?;
            safe_path(&path, root)?;
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(Error::new("发行包不允许符号链接"));
            }
            total = total
                .checked_add(entry.size())
                .ok_or_else(|| Error::new("发行包长度异常"))?;
            if total > MAX_UNPACKED {
                return Err(Error::new("发行包解压大小超限"));
            }
            let target = directory.join(path);
            if entry.is_dir() {
                fs::create_dir_all(&target)?;
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut out = File::create(target)?;
                std::io::copy(&mut entry, &mut out)?;
            }
        }
    } else {
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
        for (i, entry) in archive.entries()?.enumerate() {
            if i >= 20000 {
                return Err(Error::new("发行包文件过多"));
            }
            let mut entry = entry?;
            safe_path(&entry.path()?, root)?;
            if !entry.header().entry_type().is_file() && !entry.header().entry_type().is_dir() {
                return Err(Error::new("发行包不允许特殊文件或链接"));
            }
            total = total
                .checked_add(entry.size())
                .ok_or_else(|| Error::new("发行包长度异常"))?;
            if total > MAX_UNPACKED || !entry.unpack_in(directory)? {
                return Err(Error::new("发行包解压大小或路径异常"));
            }
        }
    }
    Ok(directory.join(root))
}
fn atomic_copy(source: &Path, target: &Path) -> Result<()> {
    let parent = target.parent().ok_or_else(|| Error::new("安装路径无效"))?;
    fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut File::open(source)?, staged.as_file_mut())?;
    staged
        .as_file()
        .set_permissions(fs::metadata(source)?.permissions())?;
    staged.as_file().sync_all()?;
    staged
        .persist(target)
        .map_err(|e| Error::new(format!("无法替换 {}: {e}", target.display())))?;
    Ok(())
}
/// Restore the executable if a platform-specific replacement failed after moving it aside.
fn install_runtime(
    bundle: &Path,
    executable: &Path,
    backup_dir: &Path,
    linux: bool,
    replace: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<()> {
    let candidate = bundle.join(if cfg!(windows) { "wocli.exe" } else { "wocli" });
    let backup_exe = backup_dir.join("previous-executable");
    fs::copy(executable, &backup_exe)?;
    let library = executable.parent().unwrap().join("lib/libasound.so.2");
    let backup_library = backup_dir.join("previous-libasound.so.2");
    let had_library = linux && library.exists();
    if had_library {
        fs::copy(&library, &backup_library)?;
    }
    if linux {
        atomic_copy(&bundle.join("lib/libasound.so.2"), &library)?;
    }
    if let Err(error) = replace(&candidate) {
        let restore = (|| {
            if !executable.exists() {
                atomic_copy(&backup_exe, executable)?;
            }
            if linux {
                if had_library {
                    atomic_copy(&backup_library, &library)?;
                } else if library.exists() {
                    fs::remove_file(&library)?;
                }
            }
            Ok(())
        })();
        let mut error = Error::new(format!("更新替换失败: {error}"));
        if let Err(restore_error) = restore {
            let restore_error: Error = restore_error;
            error.details.push(format!("恢复失败: {restore_error}"));
        }
        return Err(error);
    }
    Ok(())
}
fn verify_binary(binary: &Path, version: &Version) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(binary, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("/usr/bin/codesign")
            .args(["-s", "-", "--force"])
            .arg(binary)
            .output()?;
        if !status.status.success() {
            return Err(Error::new("新版本的 macOS 签名失败"));
        }
    }
    let output = Command::new(binary).arg("--version").output()?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim() != format!("wocli-music {version}")
    {
        return Err(Error::new("新版本无法启动或版本号不匹配，未替换当前程序"));
    }
    Ok(())
}
fn restart(executable: &Path, paths: &Paths, daemon: bool, tray: bool) -> String {
    for (enabled, command) in [(daemon, "daemon"), (tray, "tray")] {
        if !enabled {
            continue;
        }
        let result = Command::new(executable)
            .args([command, "start", "--home"])
            .arg(&paths.home)
            .env("WOCLI_RUNTIME_DIR", &paths.runtime)
            .output();
        if !result.is_ok_and(|output| output.status.success()) {
            return format!("更新成功，但 {command} 重启失败；请手动运行 wocli {command} start");
        }
    }
    String::new()
}
pub fn run(paths: &Paths, check: bool, force: bool, direct: bool) -> Result<UpdateReport> {
    let platform = platform(std::env::consts::OS, std::env::consts::ARCH)?;
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(!direct)
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("wocli-updater/", env!("CARGO_PKG_VERSION")))
        .build();
    let (latest_url, download_base) =
        distribution(std::env::var("WOCLI_DOWNLOAD_BASE").ok().as_deref())?;
    let metadata = read_limited(
        response(&agent, &latest_url)?.into_reader(),
        2 * 1024 * 1024,
    )?;
    let release: Release = serde_json::from_slice(&metadata)?;
    let latest = release.version()?;
    let current = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
    let executable = std::env::current_exe()?.canonicalize()?;
    let mut report = UpdateReport {
        status: "up_to_date".into(),
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        executable: executable.clone(),
        release_url: format!(
            "https://github.com/{REPO}/releases/tag/{}",
            release.tag_name
        ),
        warning: String::new(),
    };
    if latest < current || (latest == current && !force) {
        return Ok(report);
    }
    let zip = platform.starts_with("windows");
    let root = format!("wocli-music-{latest}-{platform}");
    let name = format!("{root}.{}", if zip { "zip" } else { "tar.gz" });
    let archive_url = release.asset(&download_base, &name)?;
    let sums_url = release.asset(&download_base, "SHA256SUMS")?;
    if check {
        report.status = "available".into();
        return Ok(report);
    }
    let parent = executable
        .parent()
        .ok_or_else(|| Error::new("无法确定安装目录"))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(".wocli-update.lock"))
        .map_err(|e| Error::new(format!("安装目录不可写，请使用有写权限的账户更新: {e}")))?;
    lock.try_lock_exclusive()
        .map_err(|_| Error::new("另一个更新正在进行"))?;
    let stage = tempfile::Builder::new()
        .prefix(".wocli-update-")
        .tempdir_in(parent)?;
    let sums = read_limited(response(&agent, &sums_url)?.into_reader(), 128 * 1024)?;
    let expected = checksum(&String::from_utf8_lossy(&sums), &name)?;
    let archive = stage.path().join(&name);
    if download(&agent, &archive_url, &archive)? != expected {
        return Err(Error::new("SHA256 校验失败，未替换当前程序"));
    }
    let bundle = unpack(&archive, stage.path(), &root, zip)?;
    let candidate = bundle.join(if zip { "wocli.exe" } else { "wocli" });
    verify_binary(&candidate, &latest)?;
    let daemon_running = ipc::request(paths, json!({"op":"status"}), false)?.is_some();
    let tray_running = tray::status(paths)?;
    if tray_running {
        tray::stop(paths)?;
    }
    if daemon_running {
        ipc::request(paths, json!({"op":"shutdown"}), false)?;
    }
    let installed = install_runtime(
        &bundle,
        &executable,
        stage.path(),
        platform.starts_with("linux"),
        |path| self_replace::self_replace(path),
    );
    if let Err(mut error) = installed {
        if !error.details.is_empty() {
            error
                .details
                .push(format!("恢复文件保留在 {}", stage.keep().display()));
        } else {
            let _ = restart(&executable, paths, daemon_running, tray_running);
        }
        return Err(error);
    }
    report.status = "updated".into();
    report.warning = restart(&executable, paths, daemon_running, tray_running);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        let install = dir.path().join("installed");
        fs::create_dir_all(bundle.join("lib")).unwrap();
        fs::create_dir_all(install.join("lib")).unwrap();
        fs::write(
            bundle.join(if cfg!(windows) { "wocli.exe" } else { "wocli" }),
            b"new binary",
        )
        .unwrap();
        fs::write(bundle.join("lib/libasound.so.2"), b"new library").unwrap();
        let exe = install.join("wocli");
        fs::write(&exe, b"old binary").unwrap();
        fs::write(install.join("lib/libasound.so.2"), b"old library").unwrap();
        (dir, bundle, exe)
    }
    #[test]
    fn platform_selection_versions_and_exact_checksums() {
        assert_eq!(platform("macos", "aarch64").unwrap(), "macos-arm64");
        assert_eq!(platform("macos", "x86_64").unwrap(), "macos-x86_64");
        assert_eq!(platform("windows", "aarch64").unwrap(), "windows-x86_64");
        assert!(platform("linux", "aarch64").is_err());
        let mut release: Release =
            serde_json::from_value(json!({"tag_name":"v0.3.10", "assets":[{"name":"SHA256SUMS"}]}))
                .unwrap();
        assert!(release.version().unwrap() > Version::parse("0.3.9").unwrap());
        assert!(release.asset(DISTRIBUTION, "missing.zip").is_err());
        release.tag_name = "v0.4.0-beta.1".into();
        assert!(release.version().is_err());
        release.tag_name = "../../evil".into();
        assert!(release.version().is_err());
        let hash = "a".repeat(64);
        let sums = format!("{}  app-sources.zip\n{hash}  app.zip\n", "b".repeat(64));
        assert_eq!(checksum(&sums, "app.zip").unwrap(), hash);
        assert!(checksum(&sums, "missing.zip").is_err());
        assert!(checksum("bad  app.zip", "app.zip").is_err());
        assert!(checksum(&(sums.clone() + &sums), "app.zip").is_err());
        assert!(read_limited(&b"12345"[..], 4).is_err());
    }
    #[test]
    fn distribution_routes_metadata_and_assets_together() {
        let (latest, base) = distribution(None).unwrap();
        assert_eq!(latest, "https://wocli.yhan.fun/api/releases/latest");
        let release: Release =
            serde_json::from_value(json!({"tag_name":"v0.3.4", "assets":[{"name":"SHA256SUMS"}]}))
                .unwrap();
        assert_eq!(
            release.asset(&base, "SHA256SUMS").unwrap(),
            "https://wocli.yhan.fun/releases/download/v0.3.4/SHA256SUMS"
        );
        let (latest, base) = distribution(Some("github")).unwrap();
        assert_eq!(
            latest,
            format!("https://api.github.com/repos/{REPO}/releases/latest")
        );
        assert!(release
            .asset(&base, "SHA256SUMS")
            .unwrap()
            .starts_with("https://github.com/"));
        assert_eq!(
            distribution(Some("https://mirror.example/dist/"))
                .unwrap()
                .1,
            "https://mirror.example/dist"
        );
        for invalid in [
            "http://mirror.example",
            "https://user:secret@mirror.example",
            "https://mirror.example?token=x",
            "https://mirror.example/#x",
            "invalid",
        ] {
            assert!(distribution(Some(invalid)).is_err());
        }
    }
    #[test]
    fn failure_restores_library_and_a_moved_executable() {
        for move_executable in [false, true] {
            let (dir, bundle, exe) = fixture();
            let result = install_runtime(&bundle, &exe, dir.path(), true, |_| {
                assert_eq!(
                    fs::read(exe.parent().unwrap().join("lib/libasound.so.2")).unwrap(),
                    b"new library"
                );
                if move_executable {
                    fs::remove_file(&exe)?;
                }
                Err(std::io::Error::other("replacement failed"))
            });
            let error = result.unwrap_err();
            assert!(error.details.is_empty(), "{error:?}");
            assert_eq!(fs::read(&exe).unwrap(), b"old binary");
            assert_eq!(
                fs::read(exe.parent().unwrap().join("lib/libasound.so.2")).unwrap(),
                b"old library"
            );
        }
    }
    #[test]
    fn successful_update_replaces_binary_and_linux_library() {
        let (dir, bundle, exe) = fixture();
        install_runtime(&bundle, &exe, dir.path(), true, |source| {
            fs::copy(source, &exe)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new binary");
        assert_eq!(
            fs::read(exe.parent().unwrap().join("lib/libasound.so.2")).unwrap(),
            b"new library"
        );
    }
    #[test]
    fn archives_unpack_only_inside_the_expected_bundle() {
        use zip::write::SimpleFileOptions;
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("test.zip");
        for name in ["bundle/wocli.exe", "../escape", "other/wocli.exe"] {
            let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(b"binary").unwrap();
            zip.finish().unwrap();
            assert_eq!(
                unpack(&archive, &dir.path().join("out"), "bundle", true).is_ok(),
                name == "bundle/wocli.exe"
            );
        }
        let archive = dir.path().join("test.tar.gz");
        let gzip = flate2::write::GzEncoder::new(
            File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gzip);
        let mut header = tar::Header::new_gnu();
        header.set_size(6);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "bundle/wocli", &b"binary"[..])
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let out = unpack(&archive, &dir.path().join("tar"), "bundle", false).unwrap();
        assert_eq!(fs::read(out.join("wocli")).unwrap(), b"binary");
        assert!(safe_path(Path::new("bundle/../escape"), "bundle").is_err());
        assert!(safe_path(Path::new("/bundle/escape"), "bundle").is_err());
    }
    #[test]
    fn download_hashes_real_http_bytes_before_replacement() {
        use std::{net::TcpListener, thread};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            assert!(stream.read(&mut buf).unwrap() > 0);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nbinary",
                )
                .unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("archive");
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .timeout(Duration::from_secs(2))
            .build();
        let hash = download(&agent, &url, &archive).unwrap();
        assert_eq!(hash, format!("{:x}", Sha256::digest(b"binary")));
        assert_eq!(fs::read(archive).unwrap(), b"binary");
        server.join().unwrap();
    }
}
