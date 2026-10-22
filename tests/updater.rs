use std::{fs, path::PathBuf, process::Command};

// Run this same test binary as a child so the replacement path exercises a
// genuinely running executable, including Windows image locks and cleanup.
#[test]
fn self_replace_child() {
    if let Some(source) = std::env::var_os("WOCLI_TEST_REPLACEMENT") {
        self_replace::self_replace(source).unwrap();
    }
}
#[test]
fn replaces_a_running_executable_and_the_new_command_starts() {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join(if cfg!(windows) {
        "updater.exe"
    } else {
        "updater"
    });
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let source = PathBuf::from(env!("CARGO_BIN_EXE_wocli"));
    let output = Command::new(&executable)
        .args(["--exact", "self_replace_child", "--nocapture"])
        .env("WOCLI_TEST_REPLACEMENT", &source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(&executable).arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("wocli-music {}", env!("CARGO_PKG_VERSION"))
    );
}
