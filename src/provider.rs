//! Transcription backends. Each one turns an audio file into segments whose
//! offsets are measured in seconds from the start of the recording.

use std::fs;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client, Response, multipart};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config::Provider;
use crate::style::{CMD, DIM};
use crate::transcript::{Segment, group_tokens};

const OPENAI_DEFAULT_MODEL: &str = "whisper-1";
const SONIOX_DEFAULT_MODEL: &str = "stt-async-v5";
const GROQ_DEFAULT_MODEL: &str = "whisper-large-v3";

/// OpenAI and Groq both reject uploads larger than this.
const SYNC_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Soniox timestamps individual tokens; join them into utterances no longer
/// than this so the output stays readable.
const SONIOX_SEGMENT_SECONDS: f64 = 15.0;

const SONIOX_POLL: Duration = Duration::from_secs(2);
const SONIOX_MAX_POLLS: u32 = 900;

pub struct Request<'a> {
    pub file: &'a Path,
    pub api_key: &'a str,
    pub model: Option<&'a str>,
    pub language: Option<&'a str>,
}

impl Request<'_> {
    /// The requested model, or the provider's default.
    pub fn model_for(&self, provider: Provider) -> &str {
        self.model.unwrap_or(match provider {
            Provider::Openai => OPENAI_DEFAULT_MODEL,
            Provider::Soniox => SONIOX_DEFAULT_MODEL,
            Provider::Groq => GROQ_DEFAULT_MODEL,
        })
    }
}

pub fn transcribe(provider: Provider, request: &Request<'_>) -> Result<Vec<Segment>> {
    match provider {
        Provider::Openai => openai(request),
        Provider::Soniox => soniox(request),
        Provider::Groq => groq(request),
    }
}

// ---------------------------------------------------------------------------
// OpenAI — and any OpenAI-compatible API (Groq follows the same shape)
// ---------------------------------------------------------------------------

fn openai(request: &Request<'_>) -> Result<Vec<Segment>> {
    let model = request.model_for(Provider::Openai);
    check_size(request, "OpenAI", Provider::Soniox)?;
    let timestamped = model.starts_with("whisper");
    progress(&format!("uploading to OpenAI ({model})"));
    let body = sync_transcription(
        request,
        "https://api.openai.com/v1/audio/transcriptions",
        model,
        timestamped,
    )?;
    Ok(segments_from(body, timestamped, model))
}

fn groq(request: &Request<'_>) -> Result<Vec<Segment>> {
    let model = request.model_for(Provider::Groq);
    check_size(request, "Groq", Provider::Soniox)?;
    let timestamped = model.starts_with("whisper");
    progress(&format!("uploading to Groq ({model})"));
    let body = sync_transcription(
        request,
        "https://api.groq.com/openai/v1/audio/transcriptions",
        model,
        timestamped,
    )?;
    Ok(segments_from(body, timestamped, model))
}

fn check_size(request: &Request<'_>, provider: &str, alternative: Provider) -> Result<()> {
    let size = fs::metadata(request.file)
        .with_context(|| format!("cannot read {}", request.file.display()))?
        .len();
    if size > SYNC_MAX_BYTES {
        bail!(
            "{} is {:.1} MB — {provider} accepts at most 25 MB.\n\n  \
             Send it to {alternative} instead, which has no such limit:\n    \
             {CMD}stt-cli transcribe {} --provider {alternative}{CMD:#}\n\n  \
             {DIM}Or split it first with ffmpeg -i … -f segment -segment_time 900 -c copy{DIM:#}",
            request.file.display(),
            size as f64 / (1024.0 * 1024.0),
            request.file.display(),
        );
    }
    Ok(())
}

/// POST a multipart audio-transcription request to an OpenAI-compatible endpoint
/// and parse the response body.
fn sync_transcription(
    request: &Request<'_>,
    api_url: &str,
    model: &str,
    timestamped: bool,
) -> Result<Body> {
    let mut form = multipart::Form::new()
        .text("model", model.to_string())
        .text(
            "response_format",
            if timestamped { "verbose_json" } else { "json" },
        )
        .file("file", request.file)
        .with_context(|| format!("cannot read {}", request.file.display()))?;
    if timestamped {
        form = form.text("timestamp_granularities[]", "segment");
    }
    if let Some(language) = request.language {
        form = form.text("language", language.to_string());
    }

    let body: Body = read_json(
        client()?
            .post(api_url)
            .bearer_auth(request.api_key)
            .multipart(form)
            .send()
            .context(format!("cannot reach {api_url}"))?,
        "transcription",
    )?;
    Ok(body)
}

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    text: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    segments: Vec<Chunk>,
}

#[derive(Deserialize)]
struct Chunk {
    start: f64,
    end: f64,
    text: String,
}

fn segments_from(body: Body, timestamped: bool, model: &str) -> Vec<Segment> {
    if body.segments.is_empty() {
        if !timestamped {
            progress(&format!(
                "{model} returns no timings — use a whisper model for per-line timestamps"
            ));
        }
        return single_segment(body.text, body.duration.unwrap_or(0.0));
    }
    clean(body.segments.into_iter().map(|chunk| Segment {
        start: chunk.start,
        end: chunk.end,
        speaker: None,
        text: chunk.text,
    }))
}

// ---------------------------------------------------------------------------
// Soniox
// ---------------------------------------------------------------------------

fn soniox(request: &Request<'_>) -> Result<Vec<Segment>> {
    const API: &str = "https://api.soniox.com/v1";
    let model = request.model_for(Provider::Soniox);
    let client = client()?;
    let key = request.api_key;

    #[derive(Deserialize)]
    struct Created {
        id: String,
    }

    progress("uploading to Soniox");
    let form = multipart::Form::new()
        .file("file", request.file)
        .with_context(|| format!("cannot read {}", request.file.display()))?;
    let file: Created = read_json(
        client
            .post(format!("{API}/files"))
            .bearer_auth(key)
            .multipart(form)
            .send()
            .context("cannot reach api.soniox.com")?,
        "soniox file upload",
    )?;

    let mut payload = serde_json::json!({ "model": model, "file_id": file.id });
    if let Some(language) = request.language {
        payload["language_hints"] = serde_json::json!([language]);
    }
    progress(&format!("transcribing with {model}"));
    let job: Created = read_json(
        client
            .post(format!("{API}/transcriptions"))
            .bearer_auth(key)
            .json(&payload)
            .send()
            .context("cannot reach api.soniox.com")?,
        "soniox transcription request",
    )?;

    let tokens = await_soniox(&client, API, key, &job.id);

    // Do not leave the recording sitting on someone else's server.
    for url in [
        format!("{API}/transcriptions/{}", job.id),
        format!("{API}/files/{}", file.id),
    ] {
        let _ = client.delete(url).bearer_auth(key).send();
    }

    Ok(group_tokens(tokens?, SONIOX_SEGMENT_SECONDS))
}

/// Poll until the job finishes, then fetch its tokens.
fn await_soniox(client: &Client, api: &str, key: &str, id: &str) -> Result<Vec<Segment>> {
    #[derive(Deserialize)]
    struct Status {
        status: String,
        #[serde(default)]
        error_message: Option<String>,
    }

    let mut reported = String::new();
    for _ in 0..SONIOX_MAX_POLLS {
        let status: Status = read_json(
            client
                .get(format!("{api}/transcriptions/{id}"))
                .bearer_auth(key)
                .send()
                .context("cannot reach api.soniox.com")?,
            "soniox transcription status",
        )?;
        match status.status.as_str() {
            "completed" => return soniox_tokens(client, api, key, id),
            "error" => bail!(
                "soniox could not transcribe the file: {}",
                status.error_message.unwrap_or_else(|| "no detail".into())
            ),
            waiting => {
                if reported != waiting {
                    progress(waiting);
                    reported = waiting.to_string();
                }
            }
        }
        sleep(SONIOX_POLL);
    }
    bail!(
        "soniox did not finish within {} minutes",
        SONIOX_MAX_POLLS * SONIOX_POLL.as_secs() as u32 / 60
    )
}

fn soniox_tokens(client: &Client, api: &str, key: &str, id: &str) -> Result<Vec<Segment>> {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        tokens: Vec<Token>,
    }
    #[derive(Deserialize)]
    struct Token {
        text: String,
        // Absent on translated tokens, which cannot be placed on the timeline.
        start_ms: Option<i64>,
        end_ms: Option<i64>,
        #[serde(default)]
        speaker: Option<serde_json::Value>,
    }

    let body: Body = read_json(
        client
            .get(format!("{api}/transcriptions/{id}/transcript"))
            .bearer_auth(key)
            .send()
            .context("cannot reach api.soniox.com")?,
        "soniox transcript",
    )?;
    Ok(body
        .tokens
        .into_iter()
        .filter_map(|token| {
            let start = token.start_ms? as f64 / 1000.0;
            Some(Segment {
                start,
                end: token.end_ms.map_or(start, |ms| ms as f64 / 1000.0),
                speaker: token.speaker.as_ref().and_then(speaker_label),
                text: token.text,
            })
        })
        .collect())
}

/// Soniox labels speakers with a number; some deployments use a string.
fn speaker_label(value: &serde_json::Value) -> Option<String> {
    let id = match value {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    Some(format!("Speaker {id}"))
}

fn client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30 * 60))
        .user_agent(concat!("stt-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("cannot start HTTP client")
}

fn read_json<T: DeserializeOwned>(response: Response, what: &str) -> Result<T> {
    let status = response.status();
    let body = response
        .text()
        .with_context(|| format!("{what}: response could not be read"))?;
    if !status.is_success() {
        bail!(
            "{what} failed ({}): {}",
            status.as_u16(),
            api_message(&body)
        );
    }
    serde_json::from_str(&body)
        .with_context(|| format!("{what}: unexpected response {}", clip(&body)))
}

/// Pull the human-readable part out of an error body, whatever shape it takes.
fn api_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            let direct = ["error_message", "message", "detail"]
                .iter()
                .find_map(|key| value.get(key)?.as_str().map(str::to_string));
            direct.or_else(|| Some(value.pointer("/error/message")?.as_str()?.to_string()))
        })
        .unwrap_or_else(|| clip(body))
}

fn clip(body: &str) -> String {
    let body = body.trim();
    match body.char_indices().nth(300) {
        Some((cut, _)) => format!("{}…", &body[..cut]),
        None => body.to_string(),
    }
}

fn single_segment(text: String, duration: f64) -> Vec<Segment> {
    clean(std::iter::once(Segment {
        start: 0.0,
        end: duration,
        speaker: None,
        text,
    }))
}

/// Trim provider padding and drop anything that ended up empty.
fn clean(segments: impl Iterator<Item = Segment>) -> Vec<Segment> {
    segments
        .map(|segment| Segment {
            text: segment.text.trim().to_string(),
            ..segment
        })
        .filter(|segment| !segment.text.is_empty())
        .collect()
}

fn progress(message: &str) {
    anstream::eprintln!("{DIM}→ {message}{DIM:#}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_bodies_are_reduced_to_their_message() {
        assert_eq!(
            api_message(r#"{"error":{"message":"Invalid file format.","type":"x"}}"#),
            "Invalid file format."
        );
        assert_eq!(
            api_message(r#"{"error_message":"audio too short"}"#),
            "audio too short"
        );
        // Anything unrecognised is shown as-is rather than swallowed.
        assert_eq!(api_message("502 Bad Gateway"), "502 Bad Gateway");
    }

    #[test]
    fn clip_keeps_multibyte_bodies_valid() {
        let body = "안".repeat(400);
        let clipped = clip(&body);
        assert!(clipped.ends_with('…'));
        assert_eq!(clipped.chars().count(), 301);
    }

    #[test]
    fn speakers_are_labelled_from_numbers_or_strings() {
        use serde_json::json;
        assert_eq!(speaker_label(&json!(1)).unwrap(), "Speaker 1");
        assert_eq!(speaker_label(&json!("host")).unwrap(), "Speaker host");
        assert!(speaker_label(&json!(null)).is_none());
    }

    #[test]
    fn model_defaults_per_provider() {
        let req = Request {
            file: Path::new("x"),
            api_key: "",
            model: None,
            language: None,
        };
        assert_eq!(req.model_for(Provider::Openai), "whisper-1");
        assert_eq!(req.model_for(Provider::Soniox), "stt-async-v5");
        assert_eq!(req.model_for(Provider::Groq), "whisper-large-v3");
    }
}
