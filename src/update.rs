use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const FORMULA: &str = "channprj/tap/stt-cli";
const TAP: &str = "channprj/tap";
const VERSION: &str = include_str!("../VERSION").trim_ascii();

/// Compare Headatever components numerically, accepting legacy `v` prefixes.
fn version_parts(version: &str) -> Result<(u64, u64, u64)> {
    let raw = version.strip_prefix('v').unwrap_or(version);
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() != 3
        || parts[1].len() != 6
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()))
    {
        bail!("invalid Headatever version: {version:?}");
    }
    Ok((parts[0].parse()?, parts[1].parse()?, parts[2].parse()?))
}

pub fn run_check(repo: &str) -> Result<()> {
    let latest = gh_latest_release_tag(repo)?;
    if version_parts(&latest)? > version_parts(VERSION)? {
        eprintln!("update available: {VERSION} → {latest}");
        eprintln!("run `stt-cli update` to install or upgrade via Homebrew");
    } else {
        eprintln!("already up-to-date ({VERSION})");
    }
    Ok(())
}

pub fn run_update() -> Result<()> {
    let current = std::env::current_exe()
        .context("cannot determine the current executable")?
        .canonicalize()
        .context("cannot resolve the current executable")?;
    // Query brew itself so custom prefixes and Linuxbrew work too.
    let prefix = brew_path(&["--prefix"])?;
    let cellar = brew_path(&["--cellar"])?;
    let managed = current.starts_with(cellar.join("stt-cli"));
    if !managed && is_homebrew_install(&current) {
        bail!("this executable belongs to another Homebrew installation; put its brew on PATH");
    }

    brew_run(&["update"])?;
    brew_run(&["tap", TAP])?;
    let info: BrewInfo = serde_json::from_str(&brew_output(&["info", "--json=v2", FORMULA])?)
        .context("cannot parse Homebrew formula information")?;
    let formula = info
        .formulae
        .iter()
        .find(|formula| formula.full_name == FORMULA)
        .context("Homebrew did not return the requested formula")?;
    let opt_binary = prefix.join("opt/stt-cli/bin/stt-cli");
    let linked_binary = prefix.join("bin/stt-cli");
    // An old, inactive HEAD keg must not turn a stable install into a HEAD update.
    let active_keg = opt_binary.canonicalize().ok();
    let head = formula
        .linked_keg
        .as_deref()
        .or_else(|| {
            active_keg
                .as_deref()?
                .parent()?
                .parent()?
                .file_name()?
                .to_str()
        })
        .is_some_and(|version| version.starts_with("HEAD-"));

    let mut migration = if managed {
        None
    } else {
        eprintln!("switching this installation to Homebrew ({FORMULA})...");
        Some(Migration::new(&current)?)
    };
    if current == linked_binary {
        // A standalone /usr/local/bin/stt-cli can block brew's own linking.
        // Keep a recoverable original before freeing only this executable path.
        if let Some(migration) = migration.as_mut() {
            migration.free_prefix_path()?;
            if !formula.installed.is_empty() {
                // A prior failed migration may have left brew's linked-keg record.
                brew_run(&["unlink", FORMULA])?;
            }
        }
    }

    if formula.installed.is_empty() {
        brew_run(&["install", FORMULA])?;
    } else if head {
        brew_run(&["upgrade", "--fetch-HEAD", FORMULA])?;
    } else {
        brew_run(&["upgrade", FORMULA])?;
    }
    brew_run(&["link", FORMULA])?;

    let resolved = opt_binary
        .canonicalize()
        .context("Homebrew did not install an executable at its opt path")?;
    if !resolved.starts_with(cellar.join("stt-cli")) {
        bail!("Homebrew executable does not resolve into the stt-cli Cellar");
    }
    let version = binary_version(&opt_binary)?;
    if !head {
        let stable = formula
            .versions
            .stable
            .as_deref()
            .context("no stable Homebrew version")?;
        if version_parts(&version)? < version_parts(stable)? {
            bail!(
                "Homebrew still has {version}, expected at least {stable}; check whether the formula is pinned (`brew unpin {FORMULA}`)"
            );
        }
    }
    if linked_binary.canonicalize().ok().as_ref() != Some(&resolved) {
        bail!(
            "Homebrew executable is not linked at {}",
            linked_binary.display()
        );
    }

    if let Some(migration) = migration.as_mut() {
        migration.finish(&opt_binary, current == linked_binary)?;
    }
    eprintln!("Homebrew update complete: stt-cli {version}");
    Ok(())
}

#[derive(serde::Deserialize)]
struct BrewInfo {
    formulae: Vec<BrewFormula>,
}

#[derive(serde::Deserialize)]
struct BrewFormula {
    full_name: String,
    installed: Vec<serde_json::Value>,
    linked_keg: Option<String>,
    versions: BrewVersions,
}

#[derive(serde::Deserialize)]
struct BrewVersions {
    stable: Option<String>,
}

fn brew_command(args: &[&str]) -> Command {
    let mut command = Command::new("brew");
    command
        .args(args)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_INSTALL_CLEANUP", "1");
    command
}

fn brew_run(args: &[&str]) -> Result<()> {
    eprintln!("running `brew {}`...", args.join(" "));
    let status = brew_command(args).status().context("cannot run Homebrew")?;
    if !status.success() {
        bail!(
            "`brew {}` exited with {status}; resolve the Homebrew error and retry",
            args.join(" ")
        );
    }
    Ok(())
}

fn brew_output(args: &[&str]) -> Result<String> {
    let output = brew_command(args).output().context(
        "cannot run `brew`; install Homebrew from https://brew.sh and put brew on PATH, then retry `stt-cli update`",
    )?;
    if !output.status.success() {
        bail!(
            "`brew {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)
        .context("invalid UTF-8 from Homebrew")?
        .trim()
        .to_string())
}

fn brew_path(args: &[&str]) -> Result<PathBuf> {
    let path = PathBuf::from(brew_output(args)?);
    if !path.is_absolute() {
        bail!("Homebrew returned an invalid path: {}", path.display());
    }
    path.canonicalize()
        .context("cannot resolve Homebrew directory")
}

/// Keeps a standalone executable recoverable until brew and PATH migration succeed.
struct Migration {
    current: PathBuf,
    directory: PathBuf,
    displaced: bool,
    committed: bool,
}

impl Migration {
    fn new(current: &Path) -> Result<Self> {
        let directory = current
            .parent()
            .context("executable has no parent")?
            .join(format!(".stt-cli-update-{}", std::process::id()));
        std::fs::create_dir(&directory).context("cannot create backup beside executable")?;
        let migration = Self {
            current: current.to_path_buf(),
            directory,
            displaced: false,
            committed: false,
        };
        // A hard link preserves the running executable without copying or overwriting a backup.
        std::fs::hard_link(current, migration.backup())
            .context("cannot back up standalone executable")?;
        Ok(migration)
    }

    fn backup(&self) -> PathBuf {
        self.directory.join("stt-cli")
    }

    fn free_prefix_path(&mut self) -> Result<()> {
        eprintln!(
            "original executable backed up at {}",
            self.backup().display()
        );
        std::fs::remove_file(&self.current).context("cannot free Homebrew executable path")?;
        self.displaced = true;
        Ok(())
    }

    fn finish(&mut self, target: &Path, in_prefix: bool) -> Result<()> {
        if !in_prefix {
            let link = self.directory.join("homebrew-link");
            std::os::unix::fs::symlink(target, &link).context("cannot create Homebrew link")?;
            std::fs::rename(link, &self.current)
                .context("cannot replace standalone executable with Homebrew link")?;
            self.displaced = true;
        }
        self.committed = true;
        eprintln!("now using Homebrew via {}", self.current.display());
        eprintln!("original executable kept at {}", self.backup().display());
        Ok(())
    }
}

impl Drop for Migration {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if self.displaced
            && let Err(error) = std::fs::rename(self.backup(), &self.current)
        {
            eprintln!(
                "cannot restore original executable: {error}; backup kept at {}",
                self.backup().display()
            );
            return;
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn is_homebrew_install(executable: &Path) -> bool {
    executable.ancestors().any(|directory| {
        directory.file_name().is_some_and(|name| name == "stt-cli")
            && directory
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "Cellar")
    })
}

fn binary_version(binary: &Path) -> Result<String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .context("cannot verify Homebrew binary")?;
    if !output.status.success() {
        bail!("Homebrew binary failed version check");
    }
    let output = String::from_utf8(output.stdout).context("invalid binary version output")?;
    let version = output
        .trim()
        .strip_prefix("stt-cli ")
        .context("unexpected binary version output")?;
    version_parts(version)?;
    Ok(version.to_string())
}

fn gh_latest_release_tag(repo: &str) -> Result<String> {
    let output = Command::new("gh")
        .args([
            "release", "view", "--repo", repo, "--json", "tagName", "--jq", ".tagName",
        ])
        .output()
        .context("cannot run `gh release view`; install gh and authenticate first")?;
    if !output.status.success() {
        bail!(
            "cannot check GitHub releases: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let tag = String::from_utf8(output.stdout)
        .context("invalid UTF-8 from gh")?
        .trim()
        .to_string();
    version_parts(&tag)?;
    Ok(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_accept_legacy_prefixes_and_compare_numerically() {
        assert_eq!(version_parts("v1.260818.0").unwrap(), (1, 260818, 0));
        assert_eq!(version_parts("1.260818.0").unwrap(), (1, 260818, 0));
        assert!(version_parts("1.260906.10").unwrap() > version_parts("1.260906.9").unwrap());
        assert!(version_parts("1.260818.0").unwrap() < version_parts("1.260906.0").unwrap());
        for invalid in ["", "latest", "1.2.3", "1.260906.0-extra", "1.260906.0.1"] {
            assert!(version_parts(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn homebrew_paths_are_identified_without_assuming_a_prefix() {
        assert!(is_homebrew_install(Path::new(
            "/opt/homebrew/Cellar/stt-cli/1.260906.0/bin/stt-cli"
        )));
        assert!(is_homebrew_install(Path::new(
            "/home/linuxbrew/.linuxbrew/Cellar/stt-cli/1.260906.0/bin/stt-cli"
        )));
        assert!(!is_homebrew_install(Path::new("/usr/local/bin/stt-cli")));
    }
}
