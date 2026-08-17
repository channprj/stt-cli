use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

/// The name of the release assets on GitHub.
const RELEASE_ASSET_NAME: &str = "stt-cli-macos-universal";
/// Current version from VERSION, used for comparison.
static VERSION: &str = include_str!("../VERSION").trim_ascii();

/// Check for updates or perform the update.
pub fn run_check(octo: &str) -> Result<()> {
    let current = VERSION.trim();
    match gh_latest_release_tag(octo)? {
        Some(tag) if tag != current => {
            eprintln!("update available: {current} → {tag}");
            eprintln!("run `stt-cli update` to upgrade");
        }
        Some(_) => {
            eprintln!("already up-to-date ({current})");
        }
        None => {
            eprintln!("could not check for updates");
            eprintln!("  ensure `gh` is installed and authenticated");
        }
    }
    Ok(())
}

/// Run the update: download the latest binary and replace the current one.
pub fn run_update(octo: &str) -> Result<()> {
    let current = VERSION.trim();
    let latest = gh_latest_release_tag(octo)?.filter(|t| t != current);

    let tag = match latest {
        Some(t) => t,
        None => {
            eprintln!("already up-to-date ({current})");
            return Ok(());
        }
    };

    let self_path = find_self_path()?;

    eprintln!("downloading {tag}...");

    let tmp_dir = std::env::temp_dir().join(format!("stt-cli-update-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir).context("failed to create temp directory")?;

    // Download using gh CLI
    let status = Command::new("gh")
        .args([
            "release",
            "download",
            &tag,
            "--repo",
            "channprj/stt-cli",
            "--pattern",
            RELEASE_ASSET_NAME,
            "--dir",
            tmp_dir.to_str().unwrap(),
            "--clobber",
        ])
        .status()
        .context("failed to run `gh release download`")?;

    if !status.success() {
        bail!("`gh release download` exited with status {}", status);
    }

    let downloaded = tmp_dir.join(RELEASE_ASSET_NAME);
    if !downloaded.exists() {
        bail!("downloaded asset not found at {downloaded:?}");
    }

    // Verify the downloaded binary
    let version_output = Command::new(&downloaded)
        .arg("--version")
        .output()
        .context("failed to verify downloaded binary")?;

    if !version_output.status.success() {
        bail!("downloaded binary failed version check");
    }

    let ver = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_string();
    let expected_tag = tag.trim_start_matches('v');
    if !ver.contains(expected_tag) {
        bail!("downloaded binary version mismatch: expected {expected_tag}, got {ver}");
    }

    // Ensure downloaded binary is executable
    std::fs::set_permissions(
        &downloaded,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .context("failed to make downloaded binary executable")?;

    // Atomically replace the current binary:
    // 1. Rename current → current.bak
    // 2. Move downloaded → current path
    // 3. On success, remove backup. On failure, restore.
    let backup = self_path.with_extension("bak");
    let _ = std::fs::remove_file(&backup); // cleanup any stale backup

    eprintln!("installing {tag}...");
    std::fs::rename(&self_path, &backup).context("failed to backup current binary")?;

    if let Err(e) = std::fs::copy(&downloaded, &self_path) {
        // Restore backup
        let _ = std::fs::rename(&backup, &self_path);
        bail!("failed to install binary: {e}");
    }

    // Verify installation
    if !self_path.exists() {
        // Restore backup
        let _ = std::fs::rename(&backup, &self_path);
        bail!("installed binary not found at {self_path:?}");
    }

    // Cleanup
    std::fs::set_permissions(
        &self_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .ok();
    let _ = std::fs::remove_file(&backup);
    let _ = std::fs::remove_dir_all(&tmp_dir);

    let final_ver = String::from_utf8_lossy(
        &Command::new(&self_path)
            .arg("--version")
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default(),
    )
    .trim()
    .to_string();

    eprintln!("updated to {final_ver}");
    Ok(())
}

/// Fetch the latest release tag name from GitHub using `gh` CLI.
fn gh_latest_release_tag(octo: &str) -> Result<Option<String>> {
    let output = Command::new("gh")
        .args([
            "release", "view", "--repo", octo, "--json", "tagName", "--jq", ".tagName",
        ])
        .output()
        .context("failed to run `gh release view`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("release not found") && !stderr.contains("not found") {
            eprintln!("gh warning: {stderr}");
        }
        return Ok(None);
    }

    let tag = String::from_utf8(output.stdout)
        .context("invalid UTF-8 from gh output")?
        .trim()
        .to_string();

    if tag.is_empty() {
        return Ok(None);
    }
    Ok(Some(tag))
}

/// Find the path to the currently running binary.
fn find_self_path() -> Result<PathBuf> {
    let exe =
        std::env::current_exe().context("cannot determine the path of the current executable")?;
    Ok(exe.canonicalize().unwrap_or(exe))
}
