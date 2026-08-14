//! Recovering the wall-clock moment a recording started.
//!
//! Recorders bake the start time into the file name (`20260815_143000.m4a`,
//! `Recording 2026-08-15 at 14.30.00.m4a`, `zoom-2026-08-15 14.30.mp4`, …).
//! Once that anchor is known, every offset the transcription model reports can
//! be turned into an absolute timestamp.

use std::sync::LazyLock;

use chrono::{NaiveDate, NaiveDateTime};
use regex::Regex;

/// Year-month-day, optionally followed by hour-minute[-second], with any of the
/// separators recorders like to use — or none at all.
static STAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?: ^ | \D )                        # never start mid-way through a digit run
        (?P<year>  (?: 19 | 20 ) \d{2} ) [-_.]?
        (?P<month> 0[1-9] | 1[0-2] )     [-_.]?
        (?P<day>   0[1-9] | [12]\d | 3[01] )
        (?:
            [\s_.,T-]{0,3} (?: at [\s_.-]? )?
            (?P<hour>   [01]\d | 2[0-3] ) [-_.:]?
            (?P<minute> [0-5]\d )
            (?: [-_.:]? (?P<second> [0-5]\d ) )?
        )?
    ",
    )
    .expect("start-time pattern is valid")
});

/// First valid date(-time) in `input`, or `None` when there is nothing to read.
///
/// Doubles as the parser for `--start`, so both accept the same spellings.
/// A date without a time is anchored at midnight.
pub fn parse(input: &str) -> Option<NaiveDateTime> {
    let number = |caps: &regex::Captures, name: &str| -> Option<u32> {
        caps.name(name)?.as_str().parse().ok()
    };

    STAMP.captures_iter(input).find_map(|caps| {
        let date = NaiveDate::from_ymd_opt(
            number(&caps, "year")? as i32,
            number(&caps, "month")?,
            number(&caps, "day")?,
        )?;
        date.and_hms_opt(
            number(&caps, "hour").unwrap_or(0),
            number(&caps, "minute").unwrap_or(0),
            number(&caps, "second").unwrap_or(0),
        )
    })
}

/// `HH:MM:SS` for an offset into the audio, used when no anchor is known.
pub fn offset_hms(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total % 3600) / 60,
        total % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> String {
        parse(text)
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "none".into())
    }

    #[test]
    fn reads_the_shapes_recorders_actually_produce() {
        for name in [
            "20260815_143022.m4a",
            "20260815143022.wav",
            "2026-08-15_14-30-22.mp3",
            "2026-08-15 14.30.22.m4a",
            "2026-08-15T14:30:22.flac",
            "New Recording 2026-08-15 at 14.30.22.m4a",
            "IMG_20260815_143022.mov",
            "standup__2026.08.15__14.30.22.ogg",
        ] {
            assert_eq!(at(name), "2026-08-15 14:30:22", "{name}");
        }
    }

    #[test]
    fn a_missing_time_anchors_at_midnight_and_seconds_are_optional() {
        assert_eq!(at("notes-2026-08-15.wav"), "2026-08-15 00:00:00");
        assert_eq!(at("zoom_20260815_1430.mp4"), "2026-08-15 14:30:00");
        // A trailing part number must not be mistaken for a time.
        assert_eq!(at("2026-08-15-1.wav"), "2026-08-15 00:00:00");
    }

    #[test]
    fn rejects_what_is_not_a_timestamp() {
        assert_eq!(at("meeting.m4a"), "none");
        assert_eq!(at("track-07.mp3"), "none");
        // Impossible dates are skipped, and a later valid one still wins.
        assert_eq!(at("2026-02-30.wav"), "none");
        assert_eq!(
            at("2026-02-30_and_20260815_143022.wav"),
            "2026-08-15 14:30:22"
        );
        // Long id-like digit runs are not dates.
        assert_eq!(at("id-987654321098.wav"), "none");
    }

    #[test]
    fn offsets_render_as_clock_time() {
        assert_eq!(offset_hms(0.0), "00:00:00");
        assert_eq!(offset_hms(5.4), "00:00:05");
        assert_eq!(offset_hms(3725.0), "01:02:05");
        assert_eq!(offset_hms(-1.0), "00:00:00");
    }
}
