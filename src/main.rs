mod config;
mod provider;
mod start_time;
mod style;
mod transcript;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anstream::{eprint, eprintln, println};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use config::{Config, Provider};
use style::{CMD, DIM, ERR, HEAD, OK, WARN};
use transcript::{Format, Source};

#[derive(Parser)]
#[command(
    name = "stt-cli",
    version,
    about = "Transcribe audio and stamp every line with the wall-clock time it was spoken",
    styles = style::HELP,
    subcommand_required = true,
    arg_required_else_help = true,
    after_help = examples()
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Transcribe an audio file
    #[command(visible_alias = "tr")]
    Transcribe(TranscribeArgs),
    /// Inspect and edit stored API keys
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Args)]
struct TranscribeArgs {
    /// Audio file to transcribe
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Service to transcribe with [default: whichever has a key]
    #[arg(short, long, value_name = "NAME")]
    provider: Option<Provider>,

    /// Model override [default: whisper-1, or stt-async-v5 for soniox]
    #[arg(short, long, value_name = "MODEL")]
    model: Option<String>,

    /// Spoken-language hint, e.g. ko or en
    #[arg(short, long, value_name = "CODE")]
    language: Option<String>,

    /// Shape of the transcript
    #[arg(short, long, value_enum, default_value = "text")]
    format: Format,

    /// Write the transcript here instead of stdout
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// When the recording started, if the file name does not say
    #[arg(short = 's', long, value_name = "WHEN")]
    start: Option<String>,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Store an API key (reads it from stdin when omitted)
    Set {
        /// Provider the key belongs to
        provider: Provider,
        /// The key itself — omit it to keep it out of your shell history
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

fn examples() -> String {
    format!(
        "\
{HEAD}Examples:{HEAD:#}
  {CMD}stt-cli config set openai{CMD:#}
      register a key, pasted from stdin so it stays out of your history

  {CMD}stt-cli transcribe 20260815_143000_standup.m4a{CMD:#}
      every line is stamped from the 14:30:00 in the file name

  {CMD}stt-cli transcribe rec.m4a --start \"2026-08-15 14:30\" -l ko{CMD:#}
      anchor the timeline yourself when the name carries no date

  {CMD}stt-cli transcribe long.mp3 -p soniox -f json -o long.json{CMD:#}
      no 25 MB limit, machine-readable output
"
    )
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
        Command::Transcribe(args) => transcribe(args),
        Command::Config { action } => config_command(action),
    }
}

fn transcribe(args: TranscribeArgs) -> Result<()> {
    if !args.file.is_file() {
        bail!("{} is not a readable file", args.file.display());
    }
    let name = args.file.file_name().unwrap_or_default().to_string_lossy();

    let config = config::load()?;
    let provider = match args.provider {
        Some(provider) => provider,
        None => config.default_provider()?,
    };
    let api_key = config.api_key(provider)?;
    let anchor = anchor_for(&name, args.start.as_deref())?;

    let request = provider::Request {
        file: &args.file,
        api_key: &api_key,
        model: args.model.as_deref(),
        language: args.language.as_deref(),
    };
    let model = request.model_for(provider).to_string();
    let segments = provider::transcribe(provider, &request)?;
    if segments.is_empty() {
        eprintln!("{WARN}!{WARN:#} no speech was recognised in {name}");
    }

    let rendered = transcript::render(
        &segments,
        args.format,
        anchor,
        &Source {
            file: &name,
            provider: provider.as_str(),
            model: &model,
        },
    )?;
    match args.output {
        Some(path) => {
            std::fs::write(&path, &rendered)
                .with_context(|| format!("cannot write {}", path.display()))?;
            println!(
                "{OK}✓{OK:#} {} lines written to {}",
                segments.len(),
                path.display()
            );
        }
        None => io::stdout().write_all(rendered.as_bytes())?,
    }
    Ok(())
}

/// Decide which wall-clock moment offset zero corresponds to, and say so.
fn anchor_for(name: &str, start: Option<&str>) -> Result<Option<chrono::NaiveDateTime>> {
    let (anchor, origin) = match start {
        Some(text) => {
            let parsed = start_time::parse(text).with_context(|| {
                format!("cannot read a date and time from {text:?} — try \"2026-08-15 14:30\"")
            })?;
            (Some(parsed), "--start")
        }
        None => (start_time::parse(name), "the file name"),
    };
    match anchor {
        Some(at) => eprintln!(
            "{DIM}→ recording starts {} (from {origin}){DIM:#}",
            at.format("%Y-%m-%d %H:%M:%S")
        ),
        None => {
            eprintln!("{WARN}!{WARN:#} no date or time in {name:?} — timestamps stay relative");
            eprintln!("  anchor them with {CMD}--start \"2026-08-15 14:30\"{CMD:#}");
        }
    }
    Ok(anchor)
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
