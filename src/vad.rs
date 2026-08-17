//! Voice activity detection — find the segments of an audio file that actually
//! contain speech, so transcription cost can be cut by skipping the silence.
//!
//! `stt-cli` shells out to `ffmpeg`'s `silencedetect` filter (a ubiquitous
//! dependency for anyone already handling audio). When ffmpeg is missing the
//! feature degrades gracefully: `--vad` reports a warning and transcribes the
//! whole file as before.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Default silence threshold in dB.  `silencedetect`'s own default is -30 dB;
/// -35 dB is a little more aggressive at calling quiet passages speech, which
/// keeps the transcript conservative about dropping audio.
pub const DEFAULT_THRESHOLD_DB: f64 = -35.0;

/// Default minimum silence length (seconds) before a gap counts as silence.
/// Recordings have natural micro-pauses between words that are NOT silence.
pub const DEFAULT_MIN_SILENCE: f64 = 0.5;

/// A contiguous run of speech in the original file, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechChunk {
    pub start: f64,
    pub end: f64,
}

/// Result of building a compressed (silence-free) copy of the audio.
pub struct Compressed {
    /// Temporary file with only the speech chunks, in order.
    pub path: PathBuf,
    /// Chunks with their position inside the compressed file.
    pub chunks: Vec<CompressedChunk>,
}

/// A speech chunk as it sits inside the compressed audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressedChunk {
    /// Start in the ORIGINAL file (seconds).
    pub orig_start: f64,
    /// End in the ORIGINAL file (seconds).
    pub orig_end: f64,
    /// Start inside the COMPRESSED file (seconds).
    pub comp_start: f64,
    /// End inside the COMPRESSED file (seconds).
    pub comp_end: f64,
}

/// Total audio duration in seconds, via `ffprobe`.  `None` when unknown.
pub fn duration(path: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim().parse::<f64>().ok()
}

/// Detect speech chunks using `ffmpeg`'s `silencedetect`.
///
/// Returns the non-silent intervals, merged so that gaps shorter than
/// `min_silence` do not split a sentence.  `Ok(vec![])` means "all silence",
/// which callers should treat as "nothing to transcribe".
pub fn detect_speech(path: &Path, threshold_db: f64, min_silence: f64) -> Result<Vec<SpeechChunk>> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats"])
        .args(["-i"])
        .arg(path)
        .args([
            "-af",
            &format!("silencedetect=noise={threshold_db}dB:d={min_silence}"),
            "-f",
            "null",
            "-",
        ])
        .output()
        .context("cannot run ffmpeg — is it installed and on PATH?")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "ffmpeg silencedetect failed: {}",
            stderr.lines().last().unwrap_or("no detail")
        );
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut silences: Vec<(f64, f64)> = Vec::new();
    let mut current_start: Option<f64> = None;
    for line in stderr.lines() {
        if let Some(pos) = line.find("silence_start:") {
            let value = line[pos + "silence_start:".len()..].trim();
            current_start = value.parse::<f64>().ok();
        } else if let Some(pos) = line.find("silence_end:") {
            let rest = &line[pos + "silence_end:".len()..];
            let end = rest.split('|').next().unwrap_or(rest).trim();
            if let (Some(start), Some(end)) = (current_start, end.parse::<f64>().ok()) {
                silences.push((start, end));
            }
            current_start = None;
        }
    }

    let total =
        duration(path).unwrap_or_else(|| silences.last().map(|&(_, end)| end).unwrap_or(0.0));
    let mut speech: Vec<SpeechChunk> = Vec::new();
    let mut cursor = 0.0;
    for (s, e) in silences {
        if s > cursor {
            speech.push(SpeechChunk {
                start: cursor,
                end: s,
            });
        }
        cursor = cursor.max(e);
    }
    if cursor < total {
        speech.push(SpeechChunk {
            start: cursor,
            end: total,
        });
    }

    merge_close_chunks(&mut speech, min_silence);
    Ok(speech)
}

/// Merge chunks separated by a gap shorter than `min_silence`, so a brief pause
/// inside a sentence does not produce two tiny API payloads.
fn merge_close_chunks(chunks: &mut Vec<SpeechChunk>, min_silence: f64) {
    let mut merged: Vec<SpeechChunk> = Vec::with_capacity(chunks.len());
    for chunk in chunks.drain(..) {
        if let Some(last) = merged.last_mut() {
            if chunk.start - last.end <= min_silence {
                last.end = chunk.end;
                continue;
            }
        }
        merged.push(chunk);
    }
    *chunks = merged;
}

/// Build a compressed audio file containing only the speech chunks, with a
/// small pad either side so consonants are not clipped at the cut points.
///
/// Returns the temp file path plus the chunk map needed to translate offsets
/// from compressed time back to original time.
pub fn build_compressed(path: &Path, chunks: &[SpeechChunk], padding: f64) -> Result<Compressed> {
    if chunks.is_empty() {
        bail!("no speech detected in the audio");
    }

    let out_dir = std::env::temp_dir().join("stt-cli-vad");
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("cannot create {}", out_dir.display()))?;
    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio".into());
    let out = out_dir.join(format!("{stem}.wav"));

    // Build a filter_complex that trims each chunk and concatenates them.
    let n = chunks.len();
    let mut filter = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let start = (chunk.start - padding).max(0.0);
        let end = chunk.end + padding;
        filter.push_str(&format!(
            "[0:a]atrim=start={start:.3}:end={end:.3},asetpts=PTS-STARTPTS[s{i}];"
        ));
    }
    for i in 0..n {
        filter.push_str(&format!("[s{i}]"));
    }
    filter.push_str(&format!("concat=n={n}:v=0:a=1[out]"));

    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-y", "-i"])
        .arg(path)
        .args(["-filter_complex", &filter, "-map", "[out]"])
        .arg(&out)
        .status()
        .context("cannot run ffmpeg — is it installed and on PATH?")?;
    if !status.success() {
        bail!("ffmpeg could not build the compressed audio");
    }

    // Compute where each chunk lands in compressed time.
    let mut comp_cursor = 0.0;
    let mut mapped = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let start = (chunk.start - padding).max(0.0);
        let end = chunk.end + padding;
        let len = (end - start).max(0.0);
        mapped.push(CompressedChunk {
            orig_start: chunk.start,
            orig_end: chunk.end,
            comp_start: comp_cursor,
            comp_end: comp_cursor + len,
        });
        comp_cursor += len;
    }

    Ok(Compressed {
        path: out,
        chunks: mapped,
    })
}

impl Compressed {
    /// Map an offset in compressed time back to the original timeline.
    pub fn map_to_original(&self, comp_secs: f64) -> f64 {
        let mut picked: Option<&CompressedChunk> = None;
        for chunk in &self.chunks {
            // A boundary value belongs to the NEXT chunk, not the one ending.
            if comp_secs >= chunk.comp_start && comp_secs <= chunk.comp_end {
                picked = Some(chunk);
            }
        }
        if let Some(chunk) = picked {
            let frac = if chunk.comp_end > chunk.comp_start {
                (comp_secs - chunk.comp_start) / (chunk.comp_end - chunk.comp_start)
            } else {
                0.0
            };
            return chunk.orig_start + frac * (chunk.orig_end - chunk.orig_start);
        }
        // Beyond the last chunk: pin to the end of the final chunk.
        self.chunks.last().map(|c| c.orig_end).unwrap_or(comp_secs)
    }

    /// Total compressed duration in seconds.
    #[allow(dead_code)]
    pub fn compressed_seconds(&self) -> f64 {
        self.chunks.last().map(|c| c.comp_end).unwrap_or(0.0)
    }

    /// Total original speech seconds (without padding) — what cost is based on.
    pub fn speech_seconds(&self) -> f64 {
        self.chunks.iter().map(|c| c.orig_end - c.orig_start).sum()
    }
}

/// Remove the temporary directory used for compressed audio.
pub fn cleanup() {
    let dir = std::env::temp_dir().join("stt-cli-vad");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(start: f64, end: f64) -> SpeechChunk {
        SpeechChunk { start, end }
    }

    #[test]
    fn close_chunks_are_merged() {
        let mut chunks = vec![chunk(0.0, 10.0), chunk(10.4, 20.0), chunk(30.0, 40.0)];
        merge_close_chunks(&mut chunks, 0.5);
        assert_eq!(
            chunks,
            vec![
                SpeechChunk {
                    start: 0.0,
                    end: 20.0
                },
                SpeechChunk {
                    start: 30.0,
                    end: 40.0
                }
            ]
        );
    }

    #[test]
    fn compressed_chunks_map_back_to_original_time() {
        let compressed = Compressed {
            path: PathBuf::from("/tmp/x.wav"),
            chunks: vec![
                CompressedChunk {
                    orig_start: 0.0,
                    orig_end: 10.0,
                    comp_start: 0.0,
                    comp_end: 10.0,
                },
                CompressedChunk {
                    orig_start: 20.0,
                    orig_end: 30.0,
                    comp_start: 10.0,
                    comp_end: 20.0,
                },
            ],
        };
        assert_eq!(compressed.map_to_original(0.0), 0.0);
        assert_eq!(compressed.map_to_original(5.0), 5.0);
        assert_eq!(compressed.map_to_original(10.0), 20.0);
        assert_eq!(compressed.map_to_original(15.0), 25.0);
        assert_eq!(compressed.map_to_original(25.0), 30.0); // past the end
        assert_eq!(compressed.compressed_seconds(), 20.0);
        assert_eq!(compressed.speech_seconds(), 20.0);
    }
}
