//! Credential storage: `$XDG_CONFIG_HOME/stt-cli/api.json` (`~/.config/stt-cli/api.json`).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::style::{CMD, DIM};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Provider {
    /// OpenAI audio transcriptions (`whisper-1` and friends)
    Openai,
    /// Soniox async speech-to-text
    Soniox,
    /// Groq LPU-accelerated Whisper (`whisper-large-v3`)
    Groq,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Openai => "openai",
            Provider::Soniox => "soniox",
            Provider::Groq => "groq",
        }
    }

    /// Environment variable checked before the config file.
    pub fn env_var(self) -> &'static str {
        match self {
            Provider::Openai => "OPENAI_API_KEY",
            Provider::Soniox => "SONIOX_API_KEY",
            Provider::Groq => "GROQ_API_KEY",
        }
    }

    pub fn signup_url(self) -> &'static str {
        match self {
            Provider::Openai => "https://platform.openai.com/api-keys",
            Provider::Soniox => "https://console.soniox.com",
            Provider::Groq => "https://console.groq.com/keys",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
}

impl Config {
    /// Resolved key for `provider`, or a message explaining how to register one.
    pub fn api_key(&self, provider: Provider) -> Result<String> {
        self.api_key_with_env(provider, std::env::var(provider.env_var()).ok())
    }

    fn api_key_with_env(&self, provider: Provider, from_env: Option<String>) -> Result<String> {
        from_env
            .into_iter()
            .chain(self.keys.get(provider.as_str()).cloned())
            .map(|key| key.trim().to_string())
            .find(|key| !key.is_empty())
            .ok_or_else(|| missing_key(provider))
    }

    /// Provider to use when `--provider` is omitted: the configured default,
    /// else whichever provider already has a key.
    pub fn default_provider(&self) -> Result<Provider> {
        if let Some(name) = &self.default_provider {
            return Provider::from_str(name, true).map_err(|_| {
                anyhow!(
                    "unknown default_provider {name:?} in {}",
                    config_file_hint()
                )
            });
        }
        [Provider::Openai, Provider::Soniox, Provider::Groq]
            .into_iter()
            .find(|p| self.api_key(*p).is_ok())
            .ok_or_else(|| missing_key(Provider::Openai))
    }
}

/// Config path for use inside messages, where a lookup failure must not abort.
fn config_file_hint() -> String {
    path().map(|p| p.display().to_string()).unwrap_or_default()
}

fn missing_key(provider: Provider) -> anyhow::Error {
    let file = config_file_hint();
    anyhow!(
        "no API key for {provider}.\n\n  Register one:\n    \
         {CMD}stt-cli config set {provider}{CMD:#}\n\n  \
         Or export it for this shell:\n    \
         {CMD}export {}=…{CMD:#}\n\n  \
         {DIM}Get a key at {}\n  Keys are stored in {file}{DIM:#}",
        provider.env_var(),
        provider.signup_url(),
    )
}

pub fn path() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").context("$HOME is not set")?).join(".config"),
    };
    if !base.is_absolute() {
        bail!("the configuration base directory must be an absolute path");
    }
    Ok(base.join("stt-cli").join("api.json"))
}

pub fn load() -> Result<Config> {
    load_from(&path()?)
}

fn load_from(path: &Path) -> Result<Config> {
    let dir = path.parent().context("config path has no parent")?;
    match private_directory(dir, false) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).context("cannot secure configuration directory"),
    }
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let metadata = file.metadata()?;
    check_regular_file(&metadata)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    // Serde errors can contain the offending value, including a pasted key.
    serde_json::from_str(&text).map_err(|e| {
        anyhow!(
            "invalid JSON in {} at line {}, column {}",
            path.display(),
            e.line(),
            e.column()
        )
    })
}

pub fn save(config: &Config) -> Result<()> {
    save_to(config, &path()?)
}

fn save_to(config: &Config, path: &Path) -> Result<()> {
    let dir = path.parent().context("config path has no parent")?;
    let directory =
        private_directory(dir, true).context("cannot secure configuration directory")?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => check_regular_file(&metadata)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("cannot inspect configuration file"),
    }
    let mut json = serde_json::to_string_pretty(config)?;
    json.push('\n');
    // NamedTempFile starts at 0600; rename never follows the destination link.
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    temporary.write_all(json.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    directory.sync_all()?;
    Ok(())
}

fn check_regular_file(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.is_file() || metadata.nlink() != 1 {
        bail!("configuration must be a regular file with no symbolic or hard links");
    }
    Ok(())
}

fn private_directory(path: &Path, create: bool) -> std::io::Result<fs::File> {
    if create {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)?;
    directory.set_permissions(fs::Permissions::from_mode(0o700))?;
    Ok(directory)
}

/// `sk-proj-abc…wxyz`, safe to print.
pub fn mask(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return "•".repeat(chars.len().max(1));
    }
    format!(
        "{}…{}",
        chars[..8].iter().collect::<String>(),
        chars[chars.len() - 4..].iter().collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_private_and_replaced_atomically() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config/api.json");
        let config = Config {
            keys: BTreeMap::from([("openai".into(), "test-only-value".into())]),
            ..Config::default()
        };
        save_to(&config, &path).unwrap();
        assert_eq!(
            fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        let original = fs::File::open(&path).unwrap();
        save_to(&Config::default(), &path).unwrap();
        assert_ne!(
            original.metadata().unwrap().ino(),
            fs::metadata(&path).unwrap().ino()
        );
        assert!(load_from(&path).unwrap().keys.is_empty());
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn credential_links_are_rejected_without_modifying_their_targets() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, "keep me").unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o644)).unwrap();
        let dir = root.path().join("config");
        fs::create_dir(&dir).unwrap();
        let path = dir.join("api.json");
        for hard_link in [false, true] {
            if hard_link {
                fs::hard_link(&outside, &path).unwrap();
            } else {
                symlink(&outside, &path).unwrap();
            }
            assert!(load_from(&path).is_err());
            assert!(save_to(&Config::default(), &path).is_err());
            assert_eq!(fs::read_to_string(&outside).unwrap(), "keep me");
            assert_eq!(fs::metadata(&outside).unwrap().mode() & 0o777, 0o644);
            fs::remove_file(&path).unwrap();
        }
        let alias = root.path().join("alias");
        symlink(&dir, &alias).unwrap();
        assert!(load_from(&alias.join("api.json")).is_err());
        assert!(save_to(&Config::default(), &alias.join("api.json")).is_err());
    }

    #[test]
    fn loading_repairs_old_permissions_and_does_not_echo_invalid_values() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("api.json");
        fs::write(&path, r#"{"keys":"private-test-value"}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let error = load_from(&path).err().unwrap();
        assert!(!format!("{error:#}").contains("private-test-value"));
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn env_wins_over_file_and_blanks_are_ignored() {
        let mut config = Config::default();
        let key = |c: &Config, env: Option<&str>| {
            c.api_key_with_env(Provider::Openai, env.map(str::to_string))
        };

        assert!(key(&config, None).is_err());
        assert_eq!(key(&config, Some("from-env")).unwrap(), "from-env");

        config.keys.insert("openai".into(), " from-file ".into());
        assert_eq!(key(&config, None).unwrap(), "from-file");
        assert_eq!(key(&config, Some("from-env")).unwrap(), "from-env");
        // A blank env var must not shadow the stored key.
        assert_eq!(key(&config, Some("  ")).unwrap(), "from-file");
    }

    #[test]
    fn missing_key_message_names_both_registration_paths() {
        let err = Config::default()
            .api_key_with_env(Provider::Soniox, None)
            .unwrap_err();
        let err = err.to_string();
        assert!(err.contains("stt-cli config set soniox"), "{err}");
        assert!(err.contains("SONIOX_API_KEY"), "{err}");
    }

    #[test]
    fn mask_keeps_only_the_recognisable_ends() {
        assert_eq!(mask("sk-proj-0123456789wxyz"), "sk-proj-…wxyz");
        assert_eq!(mask("short"), "•••••");
    }
}
