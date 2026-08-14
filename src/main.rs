mod config;
mod style;

use std::io::{self, Write};
use std::process::ExitCode;

use anstream::{eprint, eprintln, println};
use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use config::{Config, Provider};
use style::{CMD, DIM, ERR, OK, WARN};

#[derive(Parser)]
#[command(
    name = "stt-cli",
    version,
    about = "Transcribe audio and stamp every line with the wall-clock time it was spoken",
    styles = style::HELP,
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect and edit stored API keys
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Store an API key (reads it from stdin when omitted)
    Set {
        /// Provider the key belongs to
        provider: Provider,
        /// The key itself — omit it to avoid leaking the key into shell history
        api_key: Option<String>,
    },
    /// Remove a stored API key
    Unset {
        /// Provider to forget
        provider: Provider,
    },
    /// List providers with masked keys
    Show,
    /// Pick the provider used when `--provider` is omitted
    Default {
        /// Provider to use by default
        provider: Provider,
    },
    /// Print the location of the credentials file
    Path,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{ERR}error:{ERR:#} {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Config { action } => config_command(action),
    }
}

fn config_command(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Set { provider, api_key } => {
            let key = match api_key {
                Some(key) => key,
                None => prompt_key(provider)?,
            };
            let key = key.trim().to_string();
            if key.is_empty() {
                bail!("empty key — nothing was saved");
            }
            let mut config = config::load()?;
            config.keys.insert(provider.as_str().to_string(), key);
            config::save(&config)?;
            println!(
                "{OK}✓{OK:#} saved {provider} key to {}",
                config::path()?.display()
            );
        }
        ConfigAction::Unset { provider } => {
            let mut config = config::load()?;
            if config.keys.remove(provider.as_str()).is_none() {
                println!("{WARN}!{WARN:#} no stored {provider} key");
                return Ok(());
            }
            config::save(&config)?;
            println!("{OK}✓{OK:#} removed {provider} key");
        }
        ConfigAction::Show => show(&config::load()?)?,
        ConfigAction::Default { provider } => {
            let mut config = config::load()?;
            config.default_provider = Some(provider.as_str().to_string());
            config::save(&config)?;
            println!("{OK}✓{OK:#} default provider is now {provider}");
        }
        ConfigAction::Path => println!("{}", config::path()?.display()),
    }
    Ok(())
}

fn show(config: &Config) -> Result<()> {
    let default = config.default_provider().ok();
    println!("{DIM}{}{DIM:#}", config::path()?.display());
    for provider in [Provider::Openai, Provider::Soniox] {
        let marker = if default == Some(provider) { "*" } else { " " };
        let source = match std::env::var(provider.env_var()) {
            Ok(key) if !key.trim().is_empty() => {
                format!(
                    "{} {DIM}(${}){DIM:#}",
                    config::mask(key.trim()),
                    provider.env_var()
                )
            }
            _ => match config.keys.get(provider.as_str()) {
                Some(key) if !key.trim().is_empty() => config::mask(key.trim()),
                _ => format!("{DIM}not set{DIM:#}"),
            },
        };
        println!(" {OK}{marker}{OK:#} {provider:<7} {source}");
    }
    if default.is_none() {
        println!("\nRegister a key with {CMD}stt-cli config set <PROVIDER>{CMD:#}");
    }
    Ok(())
}

/// Read a key from stdin so it never lands in shell history.
fn prompt_key(provider: Provider) -> Result<String> {
    eprint!("{provider} API key {DIM}(visible while typing){DIM:#}: ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line)
}
