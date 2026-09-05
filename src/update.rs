use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const RELEASE_ASSET_NAME: &str = "stt-cli-macos-universal";
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
        eprintln!("run `stt-cli update` (Homebrew: `brew upgrade channprj/tap/stt-cli`)");
    } else {
        eprintln!("already up-to-date ({VERSION})");
    }
    Ok(())
}

pub fn run_update(repo: &str) -> Result<()> {
    let current = std::env::current_exe()
        .context("cannot determine the current executable")?
        .canonicalize()
        .context("cannot resolve the current executable")?;
    if is_homebrew_install(&current) {
        bail!(
            "this installation is managed by Homebrew.\n  \
             Run `brew upgrade channprj/tap/stt-cli`\n  \
             (for a --HEAD installation, use `brew reinstall channprj/tap/stt-cli`)."
        );
    }
    if !cfg!(target_os = "macos") {
        bail!("binary updates support macOS; update using your original installation method");
    }

    let tag = gh_latest_release_tag(repo)?;
    if version_parts(&tag)? <= version_parts(VERSION)? {
        eprintln!("already up-to-date ({VERSION})");
        return Ok(());
    }

    // Stage on the executable's filesystem so the final rename is atomic.
    // create_dir refuses a pre-existing path; Drop only owns this new directory.
    let directory = current
        .parent()
        .context("executable has no parent directory")?
        .join(format!(".stt-cli-update-{}", std::process::id()));
    std::fs::create_dir(&directory).context("cannot create update directory beside executable")?;
    let temporary = DownloadDirectory(directory);

    eprintln!("downloading {tag}...");
    let status = Command::new("gh")
        .args([
            "release",
            "download",
            &tag,
            "--repo",
            repo,
            "--pattern",
            RELEASE_ASSET_NAME,
            "--dir",
        ])
        .arg(&temporary.0)
        .status()
        .context("failed to run `gh release download`")?;
    if !status.success() {
        bail!("`gh release download` exited with {status}");
    }

    let downloaded = temporary.0.join(RELEASE_ASSET_NAME);
    verify_download(&downloaded, &tag)?;
    // A failed rename leaves the original executable untouched.
    std::fs::rename(&downloaded, &current).context("cannot replace the current executable")?;
    eprintln!("updated to {tag}");
    Ok(())
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

fn verify_download(downloaded: &Path, tag: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // GitHub downloads are ordinary files, not executable until chmod succeeds.
    std::fs::set_permissions(downloaded, std::fs::Permissions::from_mode(0o755))
        .context("cannot make downloaded binary executable")?;
    let output = Command::new(downloaded)
        .arg("--version")
        .output()
        .context("cannot verify downloaded binary")?;
    if !output.status.success() {
        bail!("downloaded binary failed version check");
    }
    let output = String::from_utf8(output.stdout).context("invalid binary version output")?;
    let version = output
        .trim()
        .strip_prefix("stt-cli ")
        .context("unexpected binary version output")?;
    if version_parts(version)? != version_parts(tag)? {
        bail!("downloaded binary version mismatch: expected {tag}, got {version}");
    }
    Ok(())
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

struct DownloadDirectory(PathBuf);

impl Drop for DownloadDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
