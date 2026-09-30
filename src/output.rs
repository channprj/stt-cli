//! Keep transcript files private and untrusted text inert on a terminal.

use std::borrow::Cow;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Atomically replace an ordinary output file without following destination links.
/// The caller must choose a trusted parent directory.
pub fn write_file(path: &Path, text: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.nlink() != 1 => {
            bail!("output must be a regular file with no symbolic or hard links");
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("cannot inspect output file"),
    }
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // NamedTempFile creates a 0600 file even with a permissive umask. A rename
    // also prevents a failed/partial write from truncating an existing result.
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(text.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .context("cannot replace output file")?;
    Ok(())
}

pub fn write_stdout(text: &str) -> io::Result<()> {
    let stdout = io::stdout();
    write_stream(stdout.lock(), text, stdout.is_terminal())
}

fn write_stream(mut stream: impl Write, text: &str, terminal: bool) -> io::Result<()> {
    let text = if terminal {
        terminal_text(text)
    } else {
        Cow::Borrowed(text)
    };
    stream.write_all(text.as_bytes())
}

/// Preserve ordinary Unicode, tabs and line breaks; show control characters as
/// literal escapes instead of allowing cursor, clipboard or bidi manipulation.
fn terminal_text(text: &str) -> Cow<'_, str> {
    let unsafe_char = |c: char| {
        (c.is_control() && c != '\n' && c != '\t')
            || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    };
    if !text.chars().any(unsafe_char) {
        return Cow::Borrowed(text);
    }
    let mut safe = String::with_capacity(text.len());
    for c in text.chars() {
        if unsafe_char(c) {
            safe.extend(c.escape_unicode());
        } else {
            safe.push(c);
        }
    }
    Cow::Owned(safe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn new_and_replaced_transcripts_are_private_and_complete() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("transcript.txt");
        write_file(&path, "first transcript").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let original = fs::File::open(&path).unwrap();
        write_file(&path, "private replacement").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "private replacement");
        assert_ne!(
            original.metadata().unwrap().ino(),
            fs::metadata(&path).unwrap().ino()
        );
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn linked_and_special_outputs_are_rejected_without_changing_targets() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("keep.txt");
        let output = root.path().join("out.txt");
        fs::write(&target, "keep this").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        for hard_link in [false, true] {
            if hard_link {
                fs::hard_link(&target, &output).unwrap();
            } else {
                symlink(&target, &output).unwrap();
            }
            assert!(write_file(&output, "private transcript").is_err());
            assert_eq!(fs::read_to_string(&target).unwrap(), "keep this");
            assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o644);
            fs::remove_file(&output).unwrap();
        }
        fs::create_dir(&output).unwrap();
        assert!(write_file(&output, "private transcript").is_err());
        assert!(output.is_dir());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[test]
    fn terminal_controls_are_escaped_but_redirected_transcripts_stay_exact() {
        let text = "안녕\tworld\n\x1b]52;c;clipboard\x07\r\x08\u{009b}2J\u{202e}text\u{2066}";
        let mut terminal = Vec::new();
        write_stream(&mut terminal, text, true).unwrap();
        let terminal = String::from_utf8(terminal).unwrap();
        assert!(terminal.starts_with("안녕\tworld\n"));
        assert!(terminal.contains("\\u{1b}]52;c;clipboard\\u{7}"));
        assert!(terminal.contains("\\u{202e}"));
        assert!(
            !terminal
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        );
        let mut redirected = Vec::new();
        write_stream(&mut redirected, text, false).unwrap();
        assert_eq!(redirected, text.as_bytes());
    }
}
