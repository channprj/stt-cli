//! Shared colour palette for `--help` and for messages we print ourselves.
//!
//! Print through `anstream::{println, eprintln}` so colours are stripped
//! automatically when the stream is not a terminal or `NO_COLOR` is set.

use anstyle::{AnsiColor, Style};
use clap::builder::Styles;

pub const HELP: Styles = Styles::styled()
    .header(AnsiColor::BrightGreen.on_default().bold())
    .usage(AnsiColor::BrightGreen.on_default().bold())
    .literal(AnsiColor::BrightCyan.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default())
    .valid(AnsiColor::BrightGreen.on_default())
    .invalid(AnsiColor::BrightYellow.on_default())
    .error(AnsiColor::BrightRed.on_default().bold());

/// Matches the section headings clap renders in `--help`.
pub const HEAD: Style = AnsiColor::BrightGreen.on_default().bold();
pub const CMD: Style = AnsiColor::BrightCyan.on_default().bold();
pub const OK: Style = AnsiColor::BrightGreen.on_default().bold();
pub const WARN: Style = AnsiColor::BrightYellow.on_default().bold();
pub const ERR: Style = AnsiColor::BrightRed.on_default().bold();
pub const DIM: Style = Style::new().dimmed();
