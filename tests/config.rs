#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

#[test]
fn piped_keys_stay_private_even_with_a_permissive_umask() {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new("sh")
        .args(["-c", "umask 000; exec \"$@\"", "sh"])
        .arg(env!("CARGO_BIN_EXE_stt-cli"))
        .args(["config", "set", "openai"])
        .env("XDG_CONFIG_HOME", root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"test-only-credential\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for bytes in [&output.stdout, &output.stderr] {
        assert!(!String::from_utf8_lossy(bytes).contains("test-only-credential"));
    }
    let dir = root.path().join("stt-cli");
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(dir.join("api.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn relative_config_home_is_rejected() {
    let output = Command::new(env!("CARGO_BIN_EXE_stt-cli"))
        .args(["config", "path"])
        .env("XDG_CONFIG_HOME", "relative-config")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("absolute path"));
}
