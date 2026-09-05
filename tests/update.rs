#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "stt-update-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let fixture = Self(path);
        fs::copy(env!("CARGO_BIN_EXE_stt-cli"), fixture.0.join("stt-cli")).unwrap();
        fixture.script("asset", "#!/bin/sh\necho 'stt-cli 9.991231.0'\n", 0o644);
        fixture.script(
            "gh",
            r#"#!/bin/sh
if [ "$2" = view ]; then
  if [ -n "$LOOKUP_FAIL" ]; then echo 'authentication failed' >&2; exit 1; fi
  echo "${LATEST_TAG:-v9.991231.0}"
  exit 0
fi
while [ "$#" -gt 0 ]; do
  if [ "$1" = --dir ]; then shift; destination="$1"; fi
  shift
done
/bin/cp "$ASSET" "$destination/stt-cli-macos-universal"
"#,
            0o755,
        );
        fixture
    }

    fn script(&self, name: &str, body: &str, mode: u32) {
        let path = self.0.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(self.0.join("stt-cli"));
        command
            .env("PATH", &self.0)
            .env("ASSET", self.0.join("asset"))
            .env_remove("LOOKUP_FAIL")
            .env_remove("LATEST_TAG");
        command
    }

    fn unchanged_after(&self, output: &Output) {
        assert!(!output.status.success());
        assert_eq!(
            fs::read(self.0.join("stt-cli")).unwrap(),
            fs::read(env!("CARGO_BIN_EXE_stt-cli")).unwrap()
        );
        assert!(fs::read_dir(&self.0).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".stt-cli-update-")
        }));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn failed_lookup_is_not_reported_as_up_to_date() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["update", "--check"])
        .env("LOOKUP_FAIL", "1")
        .output()
        .unwrap();
    fixture.unchanged_after(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("authentication failed"));
}

#[test]
fn equal_and_older_release_tags_do_not_offer_an_update() {
    let fixture = Fixture::new();
    for tag in [
        format!(
            "v{}",
            include_str!("../VERSION").trim().trim_start_matches('v')
        ),
        "v0.010101.0".to_string(),
    ] {
        let output = fixture
            .command()
            .args(["update", "--check"])
            .env("LATEST_TAG", tag)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("already up-to-date"));
    }
}

#[test]
#[cfg(target_os = "macos")]
fn update_makes_download_executable_and_replaces_only_after_verification() {
    let fixture = Fixture::new();
    let output = fixture.command().arg("update").output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    let version = fixture.command().arg("--version").output().unwrap();
    assert_eq!(version.stdout, b"stt-cli 9.991231.0\n");
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 3);
}

#[test]
#[cfg(target_os = "macos")]
fn mismatched_binary_preserves_the_original_and_cleans_downloads() {
    let fixture = Fixture::new();
    fixture.script("asset", "#!/bin/sh\necho 'stt-cli 9.991231.10'\n", 0o644);
    let output = fixture.command().arg("update").output().unwrap();
    fixture.unchanged_after(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("version mismatch"));
}
