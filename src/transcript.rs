//! Transcription results and how they are written out.

use chrono::{NaiveDateTime, TimeDelta};
use clap::ValueEnum;
use serde::Serialize;

use crate::start_time::offset_hms;

/// One utterance, positioned by its offset in seconds from the start of the audio.
#[derive(Debug, Clone)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Format {
    /// One timestamped line per utterance
    Text,
    /// Machine-readable, keeps both the offset and the absolute time
    Json,
}

/// What produced a transcript, for the JSON header.
pub struct Source<'a> {
    pub file: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
}

/// Absolute moment an offset lands on, given the anchor from the file name.
fn absolute(anchor: Option<NaiveDateTime>, offset: f64) -> Option<NaiveDateTime> {
    let delta = TimeDelta::try_milliseconds((offset * 1000.0).round() as i64)?;
    anchor?.checked_add_signed(delta)
}

pub fn render(
    segments: &[Segment],
    format: Format,
    anchor: Option<NaiveDateTime>,
    source: &Source<'_>,
) -> anyhow::Result<String> {
    match format {
        Format::Text => Ok(to_text(segments, anchor)),
        Format::Json => to_json(segments, anchor, source),
    }
}

/// `[2026-08-15 14:30:05] Speaker 1: …`, falling back to `[00:00:05] …` when the
/// file name carried no start time.
fn to_text(segments: &[Segment], anchor: Option<NaiveDateTime>) -> String {
    let mut out = String::new();
    for segment in segments {
        let stamp = match absolute(anchor, segment.start) {
            Some(at) => at.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => offset_hms(segment.start),
        };
        out.push_str(&format!("[{stamp}] "));
        if let Some(speaker) = &segment.speaker {
            out.push_str(&format!("{speaker}: "));
        }
        out.push_str(&segment.text);
        out.push('\n');
    }
    out
}

fn to_json(
    segments: &[Segment],
    anchor: Option<NaiveDateTime>,
    source: &Source<'_>,
) -> anyhow::Result<String> {
    #[derive(Serialize)]
    struct Line {
        start: f64,
        end: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        speaker: Option<String>,
        text: String,
    }

    #[derive(Serialize)]
    struct Out<'a> {
        file: &'a str,
        provider: &'a str,
        model: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        started_at: Option<String>,
        segments: Vec<Line>,
    }

    let iso = |time: NaiveDateTime| time.format("%Y-%m-%dT%H:%M:%S").to_string();
    let out = Out {
        file: source.file,
        provider: source.provider,
        model: source.model,
        started_at: anchor.map(iso),
        segments: segments
            .iter()
            .map(|segment| Line {
                start: segment.start,
                end: segment.end,
                at: absolute(anchor, segment.start).map(iso),
                speaker: segment.speaker.clone(),
                text: segment.text.clone(),
            })
            .collect(),
    };
    let mut json = serde_json::to_string_pretty(&out)?;
    json.push('\n');
    Ok(json)
}

/// Collect token-level results into readable utterances, breaking at sentence
/// ends, speaker changes, and `max_seconds`.
///
/// Only providers that timestamp every token (Soniox) need this. Token text is
/// concatenated verbatim, because the provider puts the spacing inside the
/// tokens — which is what makes unspaced scripts come out right.
pub fn group_tokens(tokens: impl IntoIterator<Item = Segment>, max_seconds: f64) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for token in tokens {
        if token.text.trim().is_empty() {
            continue;
        }
        let joinable = segments.last().is_some_and(|open| {
            open.speaker == token.speaker
                && token.end - open.start <= max_seconds
                && !ends_sentence(&open.text)
        });
        if joinable {
            let open = segments
                .last_mut()
                .expect("joinable implies an open segment");
            open.text.push_str(&token.text);
            open.end = token.end;
        } else {
            segments.push(token);
        }
    }
    for segment in &mut segments {
        segment.text = segment.text.trim().to_string();
    }
    segments
}

fn ends_sentence(text: &str) -> bool {
    matches!(
        text.trim_end().chars().last(),
        Some('.' | '?' | '!' | '。' | '？' | '！' | '…')
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::start_time;

    fn segment(start: f64, end: f64, text: &str) -> Segment {
        Segment {
            start,
            end,
            speaker: None,
            text: text.into(),
        }
    }

    fn source() -> Source<'static> {
        Source {
            file: "20260815_143000_standup.m4a",
            provider: "openai",
            model: "whisper-1",
        }
    }

    #[test]
    fn text_lines_are_stamped_with_the_wall_clock_time() {
        let anchor = start_time::parse("20260815_143000_standup.m4a");
        let out = to_text(
            &[
                segment(5.4, 8.0, "안녕하세요"),
                segment(65.0, 70.0, "start"),
            ],
            anchor,
        );
        assert_eq!(
            out,
            "[2026-08-15 14:30:05] 안녕하세요\n[2026-08-15 14:31:05] start\n"
        );
    }

    #[test]
    fn without_an_anchor_lines_stay_relative() {
        let out = to_text(&[segment(5.4, 8.0, "hello")], None);
        assert_eq!(out, "[00:00:05] hello\n");
    }

    #[test]
    fn json_keeps_both_the_offset_and_the_absolute_time() {
        let anchor = start_time::parse("20260815_143000_standup.m4a");
        let out = to_json(&[segment(5.0, 8.0, "hello")], anchor, &source()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["started_at"], "2026-08-15T14:30:00");
        assert_eq!(parsed["segments"][0]["at"], "2026-08-15T14:30:05");
        assert_eq!(parsed["segments"][0]["start"], 5.0);
        assert_eq!(parsed["model"], "whisper-1");

        // No anchor means no invented timestamps.
        let out = to_json(&[segment(5.0, 8.0, "hello")], None, &source()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(parsed.get("started_at").is_none());
        assert!(parsed["segments"][0].get("at").is_none());
    }

    #[test]
    fn tokens_group_up_to_the_end_of_a_sentence() {
        let grouped = group_tokens(
            vec![
                segment(0.0, 0.4, "Hello"),
                segment(0.5, 0.9, " there."),
                segment(1.0, 1.4, "Next"),
                segment(1.5, 1.9, " one"),
            ],
            15.0,
        );
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].text, "Hello there.");
        assert_eq!(grouped[0].end, 0.9);
        assert_eq!(grouped[1].text, "Next one");
        assert_eq!(grouped[1].start, 1.0);
    }

    #[test]
    fn a_run_without_punctuation_is_cut_at_max_seconds() {
        let tokens = vec![
            segment(0.0, 0.4, "Hello"),
            segment(0.5, 0.9, " there"),
            segment(1.0, 1.4, " again"),
            segment(1.5, 1.9, " now"),
        ];
        assert_eq!(group_tokens(tokens.clone(), 15.0).len(), 1);

        let grouped = group_tokens(tokens, 1.0);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].text, "Hello there");
        assert_eq!(grouped[1].text, "again now");
    }

    #[test]
    fn spacing_inside_tokens_is_preserved_for_unspaced_scripts() {
        let grouped = group_tokens(
            vec![segment(0.0, 0.4, "안녕"), segment(0.5, 0.9, "하세요")],
            15.0,
        );
        assert_eq!(grouped[0].text, "안녕하세요");
    }

    #[test]
    fn a_speaker_change_starts_a_new_segment() {
        let with = |speaker: &str, start: f64, text: &str| Segment {
            start,
            end: start + 0.4,
            speaker: Some(speaker.into()),
            text: text.into(),
        };
        let grouped = group_tokens(vec![with("1", 0.0, "Hi"), with("2", 0.5, "Hi")], 15.0);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[1].speaker.as_deref(), Some("2"));
    }
}
