//! Credential storage: `$XDG_CONFIG_HOME/stt-cli/api.json` (`~/.config/stt-cli/api.json`).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
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

#[derive(Debug, Default, Serialize, Deserialize)]
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
    Ok(base.join("stt-cli").join("api.json"))
}

pub fn load() -> Result<Config> {
    let path = path()?;
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("invalid JSON in {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

pub fn save(config: &Config) -> Result<()> {
    let path = path()?;
    let dir = path.parent().expect("path always has a parent");
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut json = serde_json::to_string_pretty(config)?;
    json.push('\n');
    fs::write(&path, json).with_context(|| format!("cannot write {}", path.display()))?;
    restrict(dir, 0o700)?;
    restrict(&path, 0o600)?;
    Ok(())
}

/// Keep credentials out of reach of other local accounts.
#[cfg(unix)]
fn restrict(path: &std::path::Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("cannot chmod {}", path.display()))
}

#[cfg(not(unix))]
fn restrict(_path: &std::path::Path, _mode: u32) -> Result<()> {
    Ok(())
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
