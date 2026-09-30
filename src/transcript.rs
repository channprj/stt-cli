//! Transcription results and how they are written out.

use chrono::{NaiveDateTime, TimeDelta};
use clap::ValueEnum;
use serde::Serialize;

use crate::start_time::offset_hms_milli;

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
    /// SubRip subtitle format (HH:MM:SS,mmm)
    Srt,
    /// WebVTT subtitle format (HH:MM:SS.mmm)
    Vtt,
    /// Plain text without timestamps
    Txt,
    /// Comma-separated values
    Csv,
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
        Format::Srt => Ok(to_srt(segments)),
        Format::Vtt => Ok(to_vtt(segments)),
        Format::Txt => Ok(to_txt(segments)),
        Format::Csv => Ok(to_csv(segments, anchor)),
    }
}

/// `[2026-08-15 14:30:05] Speaker 1: …`, falling back to `[00:00:05] …` when the
/// file name carried no start time.
fn to_text(segments: &[Segment], anchor: Option<NaiveDateTime>) -> String {
    let mut out = String::new();
    for segment in segments {
        let stamp = match absolute(anchor, segment.start) {
            Some(at) => at.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => crate::start_time::offset_hms(segment.start),
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

/// SubRip subtitle format (the de-facto standard).  Uses offset-based
/// HH:MM:SS,mmm timestamps and sequential cue numbers.
fn to_srt(segments: &[Segment]) -> String {
    let mut out = String::new();
    for (i, segment) in segments.iter().enumerate() {
        let start = offset_hms_milli(segment.start, ",");
        let end = offset_hms_milli(segment.end, ",");
        out.push_str(&format!("{}\n{} --> {}\n", i + 1, start, end));
        if let Some(speaker) = &segment.speaker {
            out.push_str(&format!("{speaker}: "));
        }
        out.push_str(&segment.text);
        out.push_str("\n\n");
    }
    out
}

/// WebVTT subtitle format (HTML5 `<track>`).  Uses offset-based
/// HH:MM:SS.mmm timestamps.  No cue numbers.
fn to_vtt(segments: &[Segment]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for segment in segments {
        let start = offset_hms_milli(segment.start, ".");
        let end = offset_hms_milli(segment.end, ".");
        out.push_str(&format!("{start} --> {end}\n"));
        if let Some(speaker) = &segment.speaker {
            out.push_str(&format!("{speaker}: "));
        }
        out.push_str(&segment.text);
        out.push_str("\n\n");
    }
    out
}

/// Plain concatenated text without any timestamps or speaker labels.
fn to_txt(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        out.push_str(&segment.text);
        out.push('\n');
    }
    out
}

/// Comma-separated values with a header row.
///
/// Columns: `start`, `end`, `at` (absolute wall-clock time when an anchor is
/// known, blank otherwise), `speaker`, `text`.
fn to_csv(segments: &[Segment], anchor: Option<NaiveDateTime>) -> String {
    let mut out = String::from("start,end,at,speaker,text\n");
    for segment in segments {
        let at = absolute(anchor, segment.start)
            .map(|t| t.format("%Y-%m-%dT%H:%M:%S").to_string())
            .unwrap_or_default();
        let speaker = segment.speaker.as_deref().unwrap_or("");
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_f64(segment.start),
            csv_f64(segment.end),
            csv_str(&at),
            csv_str(speaker),
            csv_str(&segment.text),
        ));
    }
    out
}

/// Format a `f64` for CSV — compact, no trailing zeros.
fn csv_f64(v: f64) -> String {
    let s = format!("{v}");
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        if trimmed.is_empty() {
            "0".into()
        } else {
            trimmed.to_string()
        }
    } else {
        s
    }
}

/// Quote CSV delimiters and neutralize spreadsheet formulas in untrusted text.
/// JSON remains the lossless format when the leading text marker is unwanted.
fn csv_str(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let significant =
        s.trim_start_matches(|c: char| c.is_whitespace() || c.is_control() || c == '\u{feff}');
    if significant.starts_with(['=', '+', '-', '@', '＝', '＋', '－', '＠'])
        || s.starts_with(['\t', '\r', '\n'])
    {
        return format!("\"'{}\"", s.replace('"', "\"\""));
    }
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
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

    #[test]
    fn csv_formula_prefixes_are_marked_as_text() {
        for value in [
            "=1+1",
            "+1",
            "-1",
            "@SUM(A1)",
            " \t=1",
            "\r=1",
            "\n=1",
            "\u{feff}=1",
            "＝1",
            "＋1",
            "－1",
            "＠SUM(A1)",
        ] {
            assert_eq!(csv_str(value), format!("\"'{value}\""));
        }
        assert_eq!(csv_str("=SUM(1,2)\""), "\"'=SUM(1,2)\"\"\"");
        assert_eq!(csv_str("first\rsecond"), "\"first\rsecond\"");
    }

    #[test]
    fn csv_protects_both_speaker_and_text_while_json_preserves_content() {
        let segments = [Segment {
            start: 0.0,
            end: 1.0,
            speaker: Some("=1+1".into()),
            text: "@SUM(A1)".into(),
        }];
        assert_eq!(
            to_csv(&segments, None),
            "start,end,at,speaker,text\n0,1,,\"'=1+1\",\"'@SUM(A1)\"\n"
        );
        let source = Source {
            file: "test.wav",
            provider: "test",
            model: "test",
        };
        let json: serde_json::Value =
            serde_json::from_str(&to_json(&segments, None, &source).unwrap()).unwrap();
        assert_eq!(json["segments"][0]["text"], "@SUM(A1)");
        assert_eq!(json["segments"][0]["speaker"], "=1+1");
    }
    fn segment(start: f64, end: f64, text: &str) -> Segment {
        Segment {
            start,
            end,
            speaker: None,
            text: text.into(),
        }
    }

    fn segment_with_speaker(start: f64, end: f64, speaker: &str, text: &str) -> Segment {
        Segment {
            start,
            end,
            speaker: Some(speaker.into()),
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
    fn srt_uses_sequential_numbers_and_comma_milli() {
        let segments = &[
            segment(5.0, 8.2, "Good morning."),
            segment(65.0, 70.0, "Let's begin."),
        ];
        let out = to_srt(segments);
        assert_eq!(
            out,
            "1\n00:00:05,000 --> 00:00:08,200\nGood morning.\n\n\
             2\n00:01:05,000 --> 00:01:10,000\nLet's begin.\n\n"
        );
    }

    #[test]
    fn srt_includes_speaker_prefix() {
        let segments = &[segment_with_speaker(5.0, 8.2, "Speaker 1", "Hello.")];
        let out = to_srt(segments);
        assert!(out.contains("Speaker 1: Hello."));
    }

    #[test]
    fn vtt_starts_with_webvtt_header_and_uses_dot_milli() {
        let segments = &[segment(5.0, 8.2, "Good morning.")];
        let out = to_vtt(segments);
        assert!(out.starts_with("WEBVTT\n\n"));
        assert!(out.contains("00:00:05.000 --> 00:00:08.200"));
        assert!(out.contains("Good morning."));
    }

    #[test]
    fn txt_concatenates_text_without_any_timestamps_or_speakers() {
        let segments = &[
            segment_with_speaker(5.0, 8.2, "1", "Good morning."),
            segment(65.0, 70.0, "Let's begin."),
        ];
        let out = to_txt(segments);
        assert_eq!(out, "Good morning.\nLet's begin.\n");
    }

    #[test]
    fn csv_has_a_header_row_and_includes_at_when_anchor_is_known() {
        let anchor = start_time::parse("20260815_143000_standup.m4a");
        let segments = &[
            segment_with_speaker(5.0, 8.2, "Speaker 1", "Good morning, team."),
            segment(65.0, 70.0, "Let's begin."),
        ];
        let out = to_csv(segments, anchor);
        let lines: Vec<&str> = out.trim().lines().collect();
        assert_eq!(lines[0], "start,end,at,speaker,text");
        // Speaker contains comma, so it's quoted.
        assert!(
            lines[1].contains("\"Good morning, team.\""),
            "line 1: {}",
            lines[1]
        );
        assert!(lines[1].contains("Speaker 1"), "line 1: {}", lines[1]);
        assert!(
            lines[1].contains("2026-08-15T14:30:05"),
            "line 1: {}",
            lines[1]
        );
        assert!(lines[2].contains("Let's begin."), "line 2: {}", lines[2]);
    }

    #[test]
    fn csv_without_anchor_has_blank_at_column() {
        let out = to_csv(&[segment(5.0, 8.2, "hello")], None);
        assert!(out.contains(",,")); // blank at
    }

    #[test]
    fn csv_f64_removes_trailing_zeros() {
        assert_eq!(csv_f64(5.0), "5");
        assert_eq!(csv_f64(5.5), "5.5");
        assert_eq!(csv_f64(0.0), "0");
        assert_eq!(csv_f64(1.234), "1.234");
    }

    #[test]
    fn csv_str_wraps_only_when_needed() {
        assert_eq!(csv_str("hello"), "hello");
        assert_eq!(csv_str(""), "");
        assert_eq!(csv_str("he,llo"), "\"he,llo\"");
        assert_eq!(csv_str("he\"llo"), "\"he\"\"llo\"");
        assert_eq!(csv_str("he\nllo"), "\"he\nllo\"");
    }

    #[test]
    fn render_dispatches_all_formats() {
        let segments = &[segment(5.0, 8.0, "hello")];
        let src = source();
        let anchor = start_time::parse("20260815_143000_standup.m4a");

        assert!(render(segments, Format::Text, anchor, &src).is_ok());
        assert!(render(segments, Format::Json, anchor, &src).is_ok());
        assert!(render(segments, Format::Srt, anchor, &src).is_ok());
        assert!(render(segments, Format::Vtt, anchor, &src).is_ok());
        assert!(render(segments, Format::Txt, anchor, &src).is_ok());
        assert!(render(segments, Format::Csv, anchor, &src).is_ok());
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
