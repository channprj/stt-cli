#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const FORMULA: &str = "channprj/tap/stt-cli";
const NEW_VERSION: &str = "9.991231.0";

struct Fixture {
    root: PathBuf,
    executable: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "stt-update-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let fixture = Self {
            executable: root.join("legacy/bin/stt-cli"),
            root,
        };
        for directory in ["legacy/bin", "commands", "brew/bin", "brew/Cellar"] {
            fs::create_dir_all(fixture.root.join(directory)).unwrap();
        }
        fs::copy(env!("CARGO_BIN_EXE_stt-cli"), &fixture.executable).unwrap();
        fixture.script("asset", "#!/bin/sh\necho 'stt-cli 9.991231.0'\n");
        fixture.script(
            "commands/gh",
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$FIXTURE/gh.log"
if [ -n "$LOOKUP_FAIL" ]; then echo 'authentication failed' >&2; exit 1; fi
echo "${LATEST_TAG:-v9.991231.0}"
"#,
        );
        fixture.script(
            "commands/brew",
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$FIXTURE/brew.log"
if [ "$1" = "${BREW_FAIL:-}" ]; then echo 'simulated brew failure' >&2; exit 1; fi
case "$1" in
  --prefix) echo "$FIXTURE/brew" ;;
  --cellar) echo "$FIXTURE/brew/Cellar" ;;
  update|tap) ;;
  unlink) /bin/rm -f "$FIXTURE/brew/bin/stt-cli" "$FIXTURE/linked.record" ;;
  info) /bin/cat "$FIXTURE/info.json" ;;
  install|upgrade)
    if [ -z "${BREW_NO_BUILD:-}" ]; then
      keg="$FIXTURE/brew/Cellar/stt-cli/${BREW_KEG:-9.991231.0}"
      /bin/mkdir -p "$keg/bin" "$FIXTURE/brew/opt"
      /bin/cp "$FIXTURE/asset" "$keg/bin/stt-cli"
      /bin/chmod +x "$keg/bin/stt-cli"
      /bin/ln -sfn "$keg" "$FIXTURE/brew/opt/stt-cli"
    fi
    ;;
  link)
    if [ -f "$FIXTURE/linked.record" ]; then exit 0; fi
    target="$FIXTURE/brew/bin/stt-cli"
    if [ -e "$target" ] && [ ! -L "$target" ]; then
      echo 'link conflict' >&2; exit 1
    fi
    /bin/ln -sfn "$FIXTURE/brew/opt/stt-cli/bin/stt-cli" "$target"
    : > "$FIXTURE/linked.record"
    ;;
  *) echo "unexpected brew command: $*" >&2; exit 2 ;;
esac
"#,
        );
        fixture.info(&[], None);
        fixture
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.root.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn info(&self, installed: &[&str], linked: Option<&str>) {
        let installed: Vec<_> = installed
            .iter()
            .map(|version| serde_json::json!({ "version": version }))
            .collect();
        fs::write(
            self.root.join("info.json"),
            serde_json::json!({ "formulae": [{
                "full_name": FORMULA,
                "installed": installed,
                "linked_keg": linked,
                "versions": { "stable": NEW_VERSION }
            }]})
            .to_string(),
        )
        .unwrap();
    }

    fn move_executable(&mut self, relative: &str) {
        let target = self.root.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::rename(&self.executable, &target).unwrap();
        self.executable = target;
    }

    fn managed(&mut self, keg: &str) {
        self.move_executable(&format!("brew/Cellar/stt-cli/{keg}/bin/stt-cli"));
        fs::create_dir_all(self.root.join("brew/opt")).unwrap();
        symlink(
            self.executable.parent().unwrap().parent().unwrap(),
            self.root.join("brew/opt/stt-cli"),
        )
        .unwrap();
        symlink(
            self.root.join("brew/opt/stt-cli/bin/stt-cli"),
            self.root.join("brew/bin/stt-cli"),
        )
        .unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .env("PATH", self.root.join("commands"))
            .env("FIXTURE", &self.root);
        for key in [
            "LOOKUP_FAIL",
            "LATEST_TAG",
            "BREW_FAIL",
            "BREW_KEG",
            "BREW_NO_BUILD",
        ] {
            command.env_remove(key);
        }
        command
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("brew.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn assert_unchanged(&self, output: &Output) {
        assert!(!output.status.success(), "{output:?}");
        assert!(!self.executable.is_symlink());
        assert_eq!(
            fs::read(&self.executable).unwrap(),
            fs::read(env!("CARGO_BIN_EXE_stt-cli")).unwrap()
        );
        assert!(self.backups().is_empty());
    }

    fn backups(&self) -> Vec<PathBuf> {
        fs::read_dir(self.executable.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".stt-cli-update-")
            })
            .collect()
    }

    fn assert_migrated(&self, output: &Output) {
        assert!(output.status.success(), "{output:?}");
        assert!(self.executable.is_symlink());
        assert_eq!(
            self.executable.canonicalize().unwrap(),
            self.root
                .join("brew/opt/stt-cli/bin/stt-cli")
                .canonicalize()
                .unwrap()
        );
        let backups = self.backups();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read(backups[0].join("stt-cli")).unwrap(),
            fs::read(env!("CARGO_BIN_EXE_stt-cli")).unwrap()
        );
        assert_eq!(
            self.command().arg("--version").output().unwrap().stdout,
            b"stt-cli 9.991231.0\n"
        );
        assert!(!self.root.join("gh.log").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
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
    fixture.assert_unchanged(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("authentication failed"));
    assert!(fixture.calls().is_empty());
}

#[test]
fn check_compares_releases_without_running_homebrew_or_migrating() {
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
    let output = fixture.command().args(["up", "--check"]).output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("install or upgrade via Homebrew"));
    assert!(fixture.calls().is_empty());
    assert!(!fixture.executable.is_symlink());
}

#[test]
fn standalone_install_migrates_and_keeps_following_the_homebrew_opt_link() {
    let fixture = Fixture::new();
    let output = fixture.command().arg("up").output().unwrap();
    fixture.assert_migrated(&output);
    assert_eq!(
        fixture.calls(),
        [
            "--prefix",
            "--cellar",
            "update",
            "tap channprj/tap",
            "info --json=v2 channprj/tap/stt-cli",
            "install channprj/tap/stt-cli",
            "link channprj/tap/stt-cli",
        ]
    );
    assert_eq!(
        fs::read_link(&fixture.executable).unwrap(),
        fixture.root.join("brew/opt/stt-cli/bin/stt-cli")
    );
}

#[test]
fn standalone_with_existing_homebrew_install_is_upgraded_and_migrated() {
    let fixture = Fixture::new();
    fixture.info(&["1.260906.0"], Some("1.260906.0"));
    let output = fixture.command().arg("update").output().unwrap();
    fixture.assert_migrated(&output);
    assert!(fixture.calls().contains(&format!("upgrade {FORMULA}")));
    assert!(!fixture.calls().contains(&format!("install {FORMULA}")));
}

#[test]
fn stable_homebrew_install_ignores_an_inactive_head_keg() {
    let mut fixture = Fixture::new();
    fixture.managed("1.260906.0");
    fixture.info(&["1.260906.0", "HEAD-abcdef0"], Some("1.260906.0"));
    let output = fixture.command().arg("update").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(fixture.calls().contains(&format!("upgrade {FORMULA}")));
    assert!(
        !fixture
            .calls()
            .iter()
            .any(|call| call.contains("--fetch-HEAD"))
    );
    assert!(fixture.backups().is_empty());
    assert!(!fixture.executable.is_symlink());
}

#[test]
fn head_install_fetches_upstream_including_when_not_prefix_linked() {
    for linked in [Some("HEAD-abcdef0"), None] {
        let mut fixture = Fixture::new();
        fixture.managed("HEAD-abcdef0");
        fixture.info(&["HEAD-abcdef0"], linked);
        let output = fixture
            .command()
            .arg("update")
            .env("BREW_KEG", "HEAD-1234567")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            fixture
                .calls()
                .contains(&format!("upgrade --fetch-HEAD {FORMULA}"))
        );
        assert!(fixture.backups().is_empty());
    }
}

#[test]
fn standalone_in_homebrew_prefix_no_longer_blocks_linking() {
    let mut fixture = Fixture::new();
    fixture.move_executable("brew/bin/stt-cli");
    let output = fixture.command().arg("update").output().unwrap();
    fixture.assert_migrated(&output);
}

#[test]
fn prefix_migration_repairs_a_stale_link_record_and_restores_on_unlink_failure() {
    for fail in [false, true] {
        let mut fixture = Fixture::new();
        fixture.move_executable("brew/bin/stt-cli");
        fixture.info(&["1.260906.0"], Some("1.260906.0"));
        fs::write(fixture.root.join("linked.record"), "").unwrap();
        let mut command = fixture.command();
        command.arg("update");
        if fail {
            command.env("BREW_FAIL", "unlink");
        }
        let output = command.output().unwrap();
        if fail {
            fixture.assert_unchanged(&output);
        } else {
            fixture.assert_migrated(&output);
        }
        assert!(fixture.calls().contains(&format!("unlink {FORMULA}")));
    }
}

#[test]
fn failures_preserve_standalone_binaries_and_restore_prefix_conflicts() {
    for in_prefix in [false, true] {
        for stage in [
            "--prefix", "--cellar", "update", "tap", "info", "install", "upgrade", "link",
        ] {
            let mut fixture = Fixture::new();
            if in_prefix {
                fixture.move_executable("brew/bin/stt-cli");
            }
            if stage == "upgrade" {
                fixture.info(&["1.260906.0"], Some("1.260906.0"));
            }
            let output = fixture
                .command()
                .arg("update")
                .env("BREW_FAIL", stage)
                .output()
                .unwrap();
            fixture.assert_unchanged(&output);
            assert!(String::from_utf8_lossy(&output.stderr).contains("simulated brew failure"));
            assert_eq!(
                fixture
                    .calls()
                    .last()
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap(),
                stage
            );
        }
    }
}

#[test]
fn invalid_homebrew_binary_restores_standalone_even_after_linking() {
    for in_prefix in [false, true] {
        for body in [
            "#!/bin/sh\nexit 1\n",
            "#!/bin/sh\necho 'unrelated binary'\n",
            "#!/bin/sh\necho 'stt-cli 0.010101.0'\n",
        ] {
            let mut fixture = Fixture::new();
            if in_prefix {
                fixture.move_executable("brew/bin/stt-cli");
            }
            fixture.script("asset", body);
            let output = fixture.command().arg("update").output().unwrap();
            fixture.assert_unchanged(&output);
            assert!(!String::from_utf8_lossy(&output.stderr).contains("update complete"));
        }
    }
}

#[test]
fn missing_brew_explains_prerequisite_without_changing_binary() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.root.join("commands/brew")).unwrap();
    let output = fixture.command().arg("update").output().unwrap();
    fixture.assert_unchanged(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("https://brew.sh"));
}

#[test]
fn malformed_formula_info_stops_before_installing() {
    for body in [
        "not json",
        r#"{"formulae":[]}"#,
        r#"{"formulae":[{"full_name":"unrelated"}]}"#,
    ] {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("info.json"), body).unwrap();
        let output = fixture.command().arg("update").output().unwrap();
        fixture.assert_unchanged(&output);
        assert!(
            !fixture
                .calls()
                .iter()
                .any(|call| call.starts_with("install "))
        );
    }
}

#[test]
fn another_homebrew_cellar_is_never_replaced() {
    let mut fixture = Fixture::new();
    fixture.move_executable("other/Cellar/stt-cli/1.260906.0/bin/stt-cli");
    let output = fixture.command().arg("update").output().unwrap();
    fixture.assert_unchanged(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("another Homebrew installation"));
    assert_eq!(fixture.calls(), ["--prefix", "--cellar"]);
}

#[test]
fn an_existing_symlink_to_a_standalone_binary_keeps_working() {
    let fixture = Fixture::new();
    let alias = fixture.root.join("alias");
    symlink(&fixture.executable, &alias).unwrap();
    let mut command = fixture.command();
    // Use the symlink as argv[0] via a fresh process, preserving the isolated environment.
    let output = Command::new(&alias)
        .args(["update"])
        .envs(
            command
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        )
        .output()
        .unwrap();
    fixture.assert_migrated(&output);
    assert_eq!(fs::read_link(alias).unwrap(), fixture.executable);
    assert!(command.arg("--version").output().unwrap().status.success());
}

#[test]
fn homebrew_opt_path_must_resolve_to_its_own_cellar() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join("brew/opt/stt-cli/bin")).unwrap();
    symlink(
        fixture.root.join("asset"),
        fixture.root.join("brew/opt/stt-cli/bin/stt-cli"),
    )
    .unwrap();
    fixture.info(&[NEW_VERSION], Some(NEW_VERSION));
    let output = fixture
        .command()
        .arg("update")
        .env("BREW_NO_BUILD", "1")
        .output()
        .unwrap();
    fixture.assert_unchanged(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not resolve into"));
}

#[test]
fn migration_leaves_config_untouched() {
    let fixture = Fixture::new();
    let config = fixture.root.join("config/stt-cli/api.json");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let original = b"{\"openai\":\"test-placeholder\"}";
    fs::write(&config, original).unwrap();
    let output = fixture
        .command()
        .arg("update")
        .env("XDG_CONFIG_HOME", fixture.root.join("config"))
        .output()
        .unwrap();
    fixture.assert_migrated(&output);
    assert_eq!(fs::read(config).unwrap(), original);
}

#[test]
fn migration_also_runs_when_the_release_check_would_be_current() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .arg("update")
        .env("LATEST_TAG", include_str!("../VERSION").trim())
        .output()
        .unwrap();
    fixture.assert_migrated(&output);
}
