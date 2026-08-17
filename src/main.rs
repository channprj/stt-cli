mod config;
mod provider;
mod start_time;
mod style;
mod transcript;
mod vad;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anstream::{eprint, eprintln, println};
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use config::{Config, Provider};
use style::{CMD, DIM, ERR, HEAD, OK, WARN};
use transcript::{Format, Source};

/// Release version, kept in one place so tags, formulae and `--version` agree.
const VERSION: &str = include_str!("../VERSION").trim_ascii();

#[derive(Parser)]
#[command(
    name = "stt-cli",
    version = VERSION,
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

    /// Validate and show what would be done without actually transcribing
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Trim silence before transcribing to cut API cost (uses ffmpeg)
    #[arg(long)]
    vad: bool,

    /// Silence threshold for --vad in dB [default: -35]
    #[arg(long, value_name = "DB", default_value_t = vad::DEFAULT_THRESHOLD_DB)]
    vad_threshold: f64,

    /// Minimum silence (seconds) before a gap counts as silence [default: 0.5]
    #[arg(long, value_name = "SECONDS", default_value_t = vad::DEFAULT_MIN_SILENCE)]
    vad_min_silence: f64,
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

  {CMD}stt-cli transcribe meeting.m4a -p groq -f srt -o meeting.srt{CMD:#}
      fast Whisper via Groq, export as subtitles

  {CMD}stt-cli transcribe talk.m4a -n{CMD:#}
      dry-run: show what would be done without calling an API

  {CMD}stt-cli transcribe meeting.m4a --vad{CMD:#}
      trim silence with VAD so you only pay for the speech
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
    let anchor = anchor_for(&name, args.start.as_deref())?;

    if args.dry_run {
        return dry_run(&args.file, &name, &anchor, &args);
    }

    let config = config::load()?;
    let provider = match args.provider {
        Some(provider) => provider,
        None => config.default_provider()?,
    };
    let api_key = config.api_key(provider)?;

    // VAD: trim silence for cost optimisation, then remap offsets.
    let (audio_file, remap, _compressed) = if args.vad {
        let chunks = vad::detect_speech(&args.file, args.vad_threshold, args.vad_min_silence)?;
        if chunks.is_empty() {
            eprintln!("{WARN}!{WARN:#} no speech detected — nothing to transcribe");
            return Ok(());
        }
        let compressed = vad::build_compressed(&args.file, &chunks, 0.2)?;
        let orig_duration = vad::duration(&args.file).unwrap_or(0.0);
        let saved = compressed.speech_seconds();
        let pct = if orig_duration > 0.0 {
            (1.0 - saved / orig_duration) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "{DIM}→ VAD: trimmed {pct:.0}% silence ({:.1}s → {:.1}s speech){DIM:#}",
            orig_duration, saved
        );
        (compressed.path.clone(), Some(compressed), true)
    } else {
        (args.file.clone(), None, false)
    };

    let request = provider::Request {
        file: &audio_file,
        api_key: &api_key,
        model: args.model.as_deref(),
        language: args.language.as_deref(),
    };
    let model = request.model_for(provider).to_string();
    let mut segments = provider::transcribe(provider, &request)?;

    // Remap segment offsets from compressed time to original timeline.
    if let Some(ref map) = remap {
        for seg in &mut segments {
            seg.start = map.map_to_original(seg.start);
            seg.end = map.map_to_original(seg.end);
        }
    }

    // Clean up temp files.
    vad::cleanup();

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

/// Print a summary of what would be done without calling any API.
fn dry_run(
    file: &std::path::Path,
    name: &str,
    anchor: &Option<chrono::NaiveDateTime>,
    args: &TranscribeArgs,
) -> Result<()> {
    let meta = std::fs::metadata(file)?;
    let size = meta.len();

    // Resolve provider and model for the dry-run display.
    let (provider, model) = match args.provider {
        Some(provider) => {
            let req = provider::Request {
                file,
                api_key: "",
                model: args.model.as_deref(),
                language: args.language.as_deref(),
            };
            let model = req.model_for(provider).to_string();
            (Some(provider), model)
        }
        None => {
            // Try config, fall back to OpenAI as the default guess.
            let p = config::load()
                .ok()
                .and_then(|c| c.default_provider().ok())
                .unwrap_or(Provider::Openai);
            let req = provider::Request {
                file,
                api_key: "",
                model: args.model.as_deref(),
                language: args.language.as_deref(),
            };
            let model = req.model_for(p).to_string();
            (Some(p), model)
        }
    };
    let provider_label = provider.unwrap_or(Provider::Openai);

    eprintln!("{DIM}── dry run ──────────────────────────────{DIM:#}");
    eprintln!("  file:      {CMD}{name}{CMD:#}");
    eprintln!("  size:      {}", file_size_human(size));
    eprintln!("  provider:  {provider_label}");
    eprintln!("  model:     {model}");

    // Duration and VAD cost estimate.
    if let Some(total_secs) = vad::duration(file) {
        let speech_secs = if args.vad {
            vad::detect_speech(file, args.vad_threshold, args.vad_min_silence)
                .ok()
                .map(|chunks| chunks.iter().map(|c| c.end - c.start).sum::<f64>())
                .unwrap_or(total_secs)
        } else {
            total_secs
        };
        eprintln!(
            "  duration:  {} ({})",
            hms(total_secs),
            file_size_human(size)
        );
        if args.vad {
            let pct = if total_secs > 0.0 {
                (1.0 - speech_secs / total_secs) * 100.0
            } else {
                0.0
            };
            eprintln!(
                "  speech:    {} ({:.0}% of audio)",
                hms(speech_secs),
                100.0 - pct
            );
        }
        let full_cost = estimate_cost(provider_label, &model, total_secs);
        let vad_cost = estimate_cost(provider_label, &model, speech_secs);
        if full_cost > 0.0 {
            let saving = full_cost - vad_cost;
            let suffix = if args.vad {
                format!(" (VAD saves ${:.4})", saving.max(0.0))
            } else {
                String::new()
            };
            eprintln!("  est. cost: ${:.4}{suffix}", vad_cost);
        } else {
            eprintln!("  est. cost: {DIM}n/a (free tier or unknown pricing){DIM:#}");
        }
    } else {
        eprintln!("  duration:  {DIM}unknown{DIM:#}");
    }

    if let Some(lang) = &args.language {
        eprintln!("  language:  {lang}");
    }
    match anchor {
        Some(at) => eprintln!("  anchor:    {}", at.format("%Y-%m-%d %H:%M:%S")),
        None => eprintln!("  anchor:    {WARN}none — timestamps will be relative{WARN:#}"),
    }
    eprintln!("  format:    {:?}", args.format);
    if let Some(path) = &args.output {
        eprintln!("  output:    {}", path.display());
    }
    if args.provider.is_none() {
        eprintln!("  (provider auto-detected from config; use --provider to override)");
    }
    eprintln!("{DIM}─────────────────────────────────────────{DIM:#}");
    Ok(())
}

fn file_size_human(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    for unit in UNITS {
        if size < 1024.0 || unit == &"GB" {
            if unit == &"B" {
                return format!("{size}{unit}");
            }
            return format!("{size:.1}{unit}", size = size);
        }
        size /= 1024.0;
    }
    format!("{bytes}B")
}

/// `HH:MM:SS` (or `MM:SS` for sub-hour) for display in dry-run.
fn hms(total_secs: f64) -> String {
    let total = total_secs.max(0.0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Estimated transcription cost in USD for a provider/model over `seconds` of
/// audio.  Returns 0.0 for free tiers or unknown pricing (callers treat that
/// as "n/a").
///
/// Pricing as of August 2026 (approximate, per audio minute):
/// - OpenAI whisper-1:      $0.006/min
/// - Groq whisper-large-v3: $0.04/hour ≈ $0.0007/min (whisper-large-v3-turbo is
///   cheaper still); free tier exists, so this is the paid-tier estimate
/// - Soniox stt-async-v5:   from $0.10/hour ≈ $0.0017/min
fn estimate_cost(provider: Provider, _model: &str, seconds: f64) -> f64 {
    let minutes = seconds / 60.0;
    let per_minute = match provider {
        Provider::Openai => 0.006,
        Provider::Groq => 0.0007,
        Provider::Soniox => 0.0017,
    };
    minutes * per_minute
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
    for provider in [Provider::Openai, Provider::Soniox, Provider::Groq] {
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
