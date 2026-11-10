use crate::model::{Error, Result};
use std::path::Path;

fn command(executable: &Path, home: &Path) -> Result<String> {
    fn quote(path: &Path) -> Result<String> {
        let text = path
            .to_str()
            .ok_or_else(|| Error::new("终端启动路径必须是有效 UTF-8"))?;
        Ok(format!("'{}'", text.replace('\'', "'\\''")))
    }
    Ok(format!(
        "{} tui --home {}",
        quote(executable)?,
        quote(home)?
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn open(paths: &crate::store::Paths) -> Result<()> {
    // Pass shell text as an argv value, never interpolate it into AppleScript.
    let script = "on run argv\n tell application \"Terminal\"\n  do script (item 1 of argv)\n  activate\n end tell\nend run";
    std::process::Command::new("osascript")
        .args(["-e", script, "--"])
        .arg(command(&std::env::current_exe()?, &paths.home)?)
        .spawn()?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};

    #[test]
    fn terminal_preserves_custom_home_and_treats_shell_metacharacters_as_data() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("player ' $(touch injected) `whoami` \"");
        let home = root
            .path()
            .join("曲库 ' \" $(touch injected) `whoami`\n第二行");
        fs::write(&executable, "#!/bin/sh\nprintf '%s\\0' \"$@\"\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let output = Command::new("/bin/sh")
            .args(["-c", &command(&executable, &home).unwrap()])
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output.stderr);
        let expected = format!("tui\0--home\0{}\0", home.display());
        assert_eq!(output.stdout, expected.as_bytes());
        assert!(!root.path().join("injected").exists());
    }
}
