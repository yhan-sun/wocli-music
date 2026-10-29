//! Configuration shared with the optional, MSIX-packaged Adaptive Card provider.
//! Windows renders lyrics in the Widgets Board after the user opens the panel.
use crate::{
    model::{Error, Result},
    store::Paths,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const CONFIG_LIMIT: u64 = 16 * 1024;
const DIAGNOSTIC_LIMIT: u64 = 64 * 1024;
const SYSTEM_MODE_MARKER: &str = "windows-lyrics-system";

#[derive(Serialize, Deserialize)]
struct Config {
    schema: u32,
    mode: String,
    enabled: bool,
    endpoint_path: PathBuf,
}

fn directory() -> Result<PathBuf> {
    if !cfg!(windows) {
        return Err(Error::new("系统 Widgets 组件仅支持 Windows 11"));
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|path| PathBuf::from(path).join("wocli-music"))
        .ok_or_else(|| Error::new("无法确定 LOCALAPPDATA，不能配置系统 Widgets"))
}

pub(crate) fn config_path() -> Result<PathBuf> {
    Ok(directory()?.join("widgets.json"))
}

pub(crate) fn action_allowed_at(config_path: &Path, paths: &Paths) -> Result<bool> {
    let endpoint = endpoint_path(paths)?;
    Ok(read_config(config_path)?
        .is_some_and(|config| config.enabled && config.endpoint_path == endpoint))
}

fn read_json(path: &Path, limit: u64) -> Result<Option<Value>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::new("Widgets 配置或诊断文件过大"));
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn read_config(path: &Path) -> Result<Option<Config>> {
    let config: Option<Config> = read_json(path, CONFIG_LIMIT)?
        .map(serde_json::from_value)
        .transpose()?;
    if config
        .as_ref()
        .is_some_and(|config| config.schema != 1 || config.mode != "system")
    {
        return Err(Error::new("不支持的 Widgets 配置格式"));
    }
    Ok(config)
}

#[cfg(any(windows, test))]
fn selected_at(config_path: &Path, paths: &Paths) -> Result<bool> {
    let endpoint = endpoint_path(paths)?;
    Ok(read_config(config_path)?.is_some_and(|config| config.endpoint_path == endpoint))
}

fn endpoint_path(paths: &Paths) -> Result<PathBuf> {
    // WOCLI_RUNTIME_DIR may be relative. The packaged provider has a different
    // working directory, so its bridge configuration must always be absolute.
    let endpoint = std::path::absolute(paths.socket())?;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(endpoint.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err(Error::new(
                "系统 Widgets 需要本机磁盘上的 WOCLI_RUNTIME_DIR",
            ));
        }
    }
    Ok(endpoint)
}

#[cfg(any(windows, test))]
fn system_mode_at(config_path: &Path, paths: &Paths) -> Result<bool> {
    // Selection of another library must not resurrect this library's old GDI
    // surface. Keep the rendering choice in the home, not just the global feed.
    if paths.home.join(SYSTEM_MODE_MARKER).try_exists()? {
        return Ok(true);
    }
    selected_at(config_path, paths)
}

pub fn system_selected(paths: &Paths) -> bool {
    #[cfg(windows)]
    {
        // Invalid configuration must not silently resurrect the custom-painted
        // surface after the user has selected the system component.
        return directory()
            .and_then(|dir| system_mode_at(&dir.join("widgets.json"), paths))
            .unwrap_or(true);
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        false
    }
}

fn configure_at(dir: &Path, paths: &Paths, enabled: bool) -> Result<()> {
    fs::create_dir_all(dir)?;
    let config_path = dir.join("widgets.json");
    let endpoint = endpoint_path(paths)?;
    fs::create_dir_all(&paths.home)?;
    fs::write(paths.home.join(SYSTEM_MODE_MARKER), "system-widget-card\n")?;
    if !enabled && read_config(&config_path)?.is_some_and(|config| config.endpoint_path != endpoint)
    {
        // The CLI also disables this home's lyrics setting. Do not disable or
        // steal another home's currently selected system widget.
        return Ok(());
    }
    let config = Config {
        schema: 1,
        mode: "system".into(),
        enabled,
        endpoint_path: endpoint,
    };
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(&mut temporary, &config)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(config_path)
        .map_err(|error| Error::new(error.to_string()))?;
    Ok(())
}

pub fn configure(paths: &Paths, enabled: bool) -> Result<()> {
    configure_at(&directory()?, paths, enabled)
}

pub fn status(paths: &Paths) -> Result<Value> {
    status_at(&directory()?, paths)
}

fn status_at(dir: &Path, paths: &Paths) -> Result<Value> {
    let config_path = dir.join("widgets.json");
    let config = read_config(&config_path)?;
    let endpoint = endpoint_path(paths)?;
    let selected = config
        .as_ref()
        .is_some_and(|config| config.endpoint_path == endpoint);
    let enabled = selected && config.as_ref().is_some_and(|config| config.enabled);
    let diagnostics = read_json(&dir.join("widgets-status.json"), DIAGNOSTIC_LIMIT)?;
    let fresh = diagnostics.as_ref().is_some_and(|diagnostics| {
        let same_endpoint = diagnostics["endpoint_path"]
            .as_str()
            .is_some_and(|path| Path::new(path) == endpoint);
        let age = diagnostics["updated_at_unix_ms"]
            .as_i64()
            .and_then(|stamp| chrono::Utc::now().timestamp_millis().checked_sub(stamp));
        diagnostics["mode"] == "system-widget-card"
            && same_endpoint
            && age.is_some_and(|age| (0..=5000).contains(&age))
    });
    Ok(json!({
        "mode": "system-widget-card",
        "surface": "widgets-board",
        "selected": selected,
        "enabled": enabled,
        "system_mode": paths.home.join(SYSTEM_MODE_MARKER).try_exists()? || selected,
        "lyrics_bar": paths.settings()?.lyrics_bar,
        "provider_responding": fresh,
        "config_path": config_path,
        "diagnostics": if fresh { diagnostics } else { None },
        "note": if !selected && config.is_some() {
            "系统组件当前连接另一曲库。此曲库不会因切换目标自动恢复自绘；使用对应 --home 执行 widgets on 可重新选择。"
        } else if fresh {
            "官方 Adaptive Card 小组件：展开 Windows 小组件面板后查看歌词；组件心跳和更新成功不代表屏幕上当前可见。"
        } else {
            "尚无此曲库的卡片组件心跳。请安装 0.4.11-alpha.2 或更新的组件，在 Windows 小组件面板添加并打开 wocli 歌词卡片。旧 Feed 组件不适用。"
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_rejects_old_feed_other_library_and_expired_heartbeats() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: temp.path().join("home"),
            runtime: temp.path().join("run"),
        };
        configure_at(temp.path(), &paths, true).unwrap();
        let mut diagnostics = json!({
            "mode":"system-widget-card", "endpoint_path":endpoint_path(&paths).unwrap(),
            "updated_at_unix_ms":chrono::Utc::now().timestamp_millis(),
        });
        let write_status = |value: &Value| {
            fs::write(
                temp.path().join("widgets-status.json"),
                serde_json::to_vec(value).unwrap(),
            )
            .unwrap()
        };
        write_status(&diagnostics);
        assert_eq!(
            status_at(temp.path(), &paths).unwrap()["provider_responding"],
            true
        );
        diagnostics["mode"] = json!("system-feed-preview");
        write_status(&diagnostics);
        let old = status_at(temp.path(), &paths).unwrap();
        assert_eq!(old["provider_responding"], false);
        assert!(old["diagnostics"].is_null());
        diagnostics["mode"] = json!("system-widget-card");
        diagnostics["endpoint_path"] = json!(temp.path().join("another-endpoint"));
        write_status(&diagnostics);
        assert_eq!(
            status_at(temp.path(), &paths).unwrap()["provider_responding"],
            false
        );
        diagnostics["endpoint_path"] = json!(endpoint_path(&paths).unwrap());
        for age in [10_000, -10_000] {
            diagnostics["updated_at_unix_ms"] = json!(chrono::Utc::now().timestamp_millis() - age);
            write_status(&diagnostics);
            assert_eq!(
                status_at(temp.path(), &paths).unwrap()["provider_responding"],
                false
            );
        }
    }

    #[test]
    fn system_mode_stays_selected_when_disabled_and_is_scoped_to_home() {
        let temp = tempfile::tempdir().unwrap();
        let first = Paths {
            home: temp.path().join("one"),
            runtime: temp.path().join("run-one"),
        };
        let second = Paths {
            home: temp.path().join("two"),
            runtime: temp.path().join("run-two"),
        };
        configure_at(temp.path(), &first, true).unwrap();
        let config = temp.path().join("widgets.json");
        assert!(selected_at(&config, &first).unwrap());
        assert!(!selected_at(&config, &second).unwrap());
        configure_at(temp.path(), &second, false).unwrap();
        assert!(selected_at(&config, &first).unwrap());
        configure_at(temp.path(), &first, false).unwrap();
        assert!(selected_at(&config, &first).unwrap());
        assert!(!read_config(&config).unwrap().unwrap().enabled);
        configure_at(temp.path(), &second, true).unwrap();
        assert!(!selected_at(&config, &first).unwrap());
        assert!(selected_at(&config, &second).unwrap());
        assert!(system_mode_at(&config, &first).unwrap());
        configure_at(temp.path(), &first, false).unwrap();
        assert!(read_config(&config).unwrap().unwrap().enabled);
        assert!(selected_at(&config, &second).unwrap());
        fs::remove_file(&config).unwrap();
        assert!(system_mode_at(&config, &first).unwrap());
        assert!(system_mode_at(&config, &second).unwrap());
    }

    #[test]
    fn relative_runtime_is_resolved_before_passing_it_to_another_process() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: temp.path().join("home"),
            runtime: PathBuf::from("relative-runtime"),
        };
        configure_at(temp.path(), &paths, true).unwrap();
        let config_path = temp.path().join("widgets.json");
        let config = read_config(&config_path).unwrap().unwrap();
        assert!(config.endpoint_path.is_absolute());
        assert_eq!(
            config.endpoint_path,
            std::env::current_dir().unwrap().join(paths.socket())
        );
        assert!(selected_at(&config_path, &paths).unwrap());
    }

    #[test]
    fn rejects_corrupt_oversized_and_future_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("widgets.json");
        assert!(read_config(&file).unwrap().is_none());
        fs::write(&file, b"{").unwrap();
        assert!(read_config(&file).is_err());
        fs::write(&file, vec![b' '; CONFIG_LIMIT as usize + 1]).unwrap();
        assert!(read_config(&file).is_err());
        fs::write(
            &file,
            br#"{"schema":2,"mode":"system","enabled":true,"endpoint_path":"test"}"#,
        )
        .unwrap();
        assert!(read_config(&file).is_err());
    }
}
