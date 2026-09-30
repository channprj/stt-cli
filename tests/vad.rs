#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture(tempfile::TempDir);

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("commands")).unwrap();
        fs::create_dir(root.path().join("tmp")).unwrap();
        fs::create_dir(root.path().join("tmp/stt-cli-vad")).unwrap();
        fs::write(root.path().join("tmp/stt-cli-vad/keep"), "another run").unwrap();
        fs::write(root.path().join("-audio.wav"), "fixture").unwrap();
        let fixture = Self(root);
        fixture.script("ffprobe", "#!/bin/sh\necho 10\n");
        fixture.script(
            "ffmpeg",
            r#"#!/bin/sh
set -eu
previous=''
restricted=0
for arg do
  if [ "$previous" = '-protocol_whitelist' ] && [ "$arg" = file ]; then restricted=1; fi
  if [ "$previous" = '-i' ]; then
    case "$arg" in /*) ;; *) exit 91 ;; esac
  fi
  previous="$arg"
done
[ "$restricted" = 1 ] || exit 92
case "$*" in *silencedetect*) exit 0 ;; esac
printf 'audio' > "$previous"
printf '%s' "$previous" > "$FIXTURE/output-$RUN_ID"
if [ "${BUILD_FAIL:-}" = 1 ]; then exit 1; fi
while [ ! -f "$FIXTURE/go" ]; do /bin/sleep 0.05; done
"#,
        );
        fixture
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.0.path().join("commands").join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn start(&self, id: &str, fail: bool) -> Child {
        Command::new(env!("CARGO_BIN_EXE_stt-cli"))
            .args([
                "transcribe",
                "--vad",
                "--provider",
                "openai",
                "--",
                "-audio.wav",
            ])
            .current_dir(self.0.path())
            .env("PATH", self.0.path().join("commands"))
            .env("TMPDIR", self.0.path().join("tmp"))
            .env("XDG_CONFIG_HOME", self.0.path().join("config"))
            // Invalid header guarantees failure before any network request.
            .env("OPENAI_API_KEY", "test\ninvalid-header")
            .env("FIXTURE", self.0.path())
            .env("RUN_ID", id)
            .env("BUILD_FAIL", if fail { "1" } else { "0" })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn output_path(&self, id: &str) -> PathBuf {
        let deadline = Instant::now() + Duration::from_secs(10);
        let record = self.0.path().join(format!("output-{id}"));
        loop {
            if let Ok(path) = fs::read_to_string(&record)
                && !path.is_empty()
            {
                return PathBuf::from(path);
            }
            assert!(Instant::now() < deadline, "ffmpeg did not run");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn concurrent_provider_errors_remove_only_their_private_audio() {
    let fixture = Fixture::new();
    let first = fixture.start("first", false);
    let second = fixture.start("second", false);
    let paths = [fixture.output_path("first"), fixture.output_path("second")];
    assert_ne!(paths[0].parent(), paths[1].parent());
    for path in &paths {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    fs::write(fixture.0.path().join("go"), "").unwrap();
    for child in [first, second] {
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("builder error"));
    }
    for path in paths {
        assert!(!path.parent().unwrap().exists());
    }
    assert!(fixture.0.path().join("tmp/stt-cli-vad/keep").exists());
}

#[test]
fn failed_ffmpeg_removes_partial_audio() {
    let fixture = Fixture::new();
    let child = fixture.start("failed", true);
    let path = fixture.output_path("failed");
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not build"));
    assert!(!path.parent().unwrap().exists());
    assert!(fixture.0.path().join("tmp/stt-cli-vad/keep").exists());
}
