//! Transcription backends. Each one turns an audio file into segments whose
//! offsets are measured in seconds from the start of the recording.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
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
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

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
        request.api_key,
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
    soniox_with_client(request, &client()?, "https://api.soniox.com/v1")
}

fn soniox_with_client(request: &Request<'_>, client: &Client, api: &str) -> Result<Vec<Segment>> {
    let model = request.model_for(Provider::Soniox);
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
            .post(format!("{api}/files"))
            .bearer_auth(key)
            .multipart(form)
            .send()
            .context("cannot reach api.soniox.com")?,
        "soniox file upload",
        key,
    )?;
    resource_url(api, "files", &file.id)?;

    let mut job_id = None;
    let tokens = (|| {
        let mut payload = serde_json::json!({ "model": model, "file_id": file.id });
        if let Some(language) = request.language {
            payload["language_hints"] = serde_json::json!([language]);
        }
        progress(&format!("transcribing with {model}"));
        let job: Created = read_json(
            client
                .post(format!("{api}/transcriptions"))
                .bearer_auth(key)
                .json(&payload)
                .send()
                .context("cannot reach api.soniox.com")?,
            "soniox transcription request",
            key,
        )?;
        resource_url(api, "transcriptions", &job.id)?;
        job_id = Some(job.id);
        await_soniox(client, api, key, job_id.as_deref().unwrap())
    })();

    // Run for every error after upload, including failed job creation.
    for warning in cleanup_soniox(client, api, key, &file.id, job_id.as_deref()) {
        anstream::eprintln!("warning: {warning}; remove retained data in your Soniox account");
    }
    Ok(group_tokens(tokens?, SONIOX_SEGMENT_SECONDS))
}

fn resource_url(api: &str, resource: &str, id: &str) -> Result<String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        bail!("Soniox returned an invalid resource ID; check your account for retained data");
    }
    Ok(format!("{api}/{resource}/{id}"))
}

fn cleanup_soniox(
    client: &Client,
    api: &str,
    key: &str,
    file: &str,
    job: Option<&str>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for (resource, id) in job
        .map(|id| ("transcriptions", id))
        .into_iter()
        .chain(std::iter::once(("files", file)))
    {
        let result = resource_url(api, resource, id).and_then(|url| {
            let response = client
                .delete(url)
                .bearer_auth(key)
                .timeout(Duration::from_secs(15))
                .send()?;
            if !response.status().is_success()
                && response.status() != reqwest::StatusCode::NOT_FOUND
            {
                bail!("HTTP {}", response.status().as_u16());
            }
            Ok(())
        });
        if result.is_err() {
            warnings.push(safe_message(
                &format!("could not delete Soniox {resource}/{id}"),
                key,
            ));
        }
    }
    warnings
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
                .get(resource_url(api, "transcriptions", id)?)
                .bearer_auth(key)
                .send()
                .map_err(reqwest::Error::without_url)
                .context("cannot reach api.soniox.com")?,
            "soniox transcription status",
            key,
        )?;
        match status.status.as_str() {
            "completed" => return soniox_tokens(client, api, key, id),
            "error" => bail!(
                "soniox could not transcribe the file: {}",
                safe_message(status.error_message.as_deref().unwrap_or("no detail"), key)
            ),
            waiting => {
                if reported != waiting {
                    progress(&safe_message(waiting, key));
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
            .get(format!(
                "{}/transcript",
                resource_url(api, "transcriptions", id)?
            ))
            .bearer_auth(key)
            .send()
            .map_err(reqwest::Error::without_url)
            .context("cannot reach api.soniox.com")?,
        "soniox transcript",
        key,
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
    client_builder()
        .https_only(true)
        .build()
        .context("cannot start HTTP client")
}

fn client_builder() -> reqwest::blocking::ClientBuilder {
    Client::builder()
        // Never replay private uploads to a redirect target.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30 * 60))
        .user_agent(concat!("stt-cli/", env!("CARGO_PKG_VERSION")))
}

fn read_json<T: DeserializeOwned>(response: Response, what: &str, key: &str) -> Result<T> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        bail!("{what}: response exceeds 64 MiB limit");
    }
    let mut body = String::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_string(&mut body)
        .with_context(|| format!("{what}: response could not be read"))?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        bail!("{what}: response exceeds 64 MiB limit");
    }
    if !status.is_success() {
        bail!(
            "{what} failed ({}): {}",
            status.as_u16(),
            api_message(&body, key)
        );
    }
    serde_json::from_str(&body).map_err(|e| {
        anyhow!(
            "{what}: invalid JSON response at line {}, column {}",
            e.line(),
            e.column()
        )
    })
}

/// Pull the human-readable part out of an error body, whatever shape it takes.
fn api_message(body: &str, key: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            let direct = ["error_message", "message", "detail"]
                .iter()
                .find_map(|key| value.get(key)?.as_str().map(str::to_string));
            direct.or_else(|| Some(value.pointer("/error/message")?.as_str()?.to_string()))
        })
        .unwrap_or_else(|| "provider returned an error response".into());
    safe_message(&message, key)
}

fn safe_message(message: &str, key: &str) -> String {
    let redacted = if key.is_empty() {
        message.to_string()
    } else {
        message.replace(key, "[redacted]")
    };
    let printable: String = redacted
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clip(&printable)
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
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    fn reply(status: u16, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn server(replies: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in replies {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "expected HTTP request did not arrive"
                            );
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                requests.push(first.trim().trim_end_matches(" HTTP/1.1").to_string());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    assert!(
                        reader.read_line(&mut line).unwrap() > 0,
                        "incomplete HTTP headers"
                    );
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, thread)
    }

    fn mock_client() -> Client {
        client_builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    #[test]
    fn uploaded_audio_is_deleted_when_job_creation_fails() {
        let (api, server) = server(vec![
            reply(200, r#"{"id":"file-1"}"#),
            reply(400, r#"{"message":"bad model"}"#),
            reply(204, ""),
        ]);
        let audio = tempfile::NamedTempFile::new().unwrap();
        let request = Request {
            file: audio.path(),
            api_key: "test-value",
            model: None,
            language: None,
        };
        assert!(soniox_with_client(&request, &mock_client(), &api).is_err());
        assert_eq!(
            server.join().unwrap(),
            [
                "POST /files",
                "POST /transcriptions",
                "DELETE /files/file-1"
            ]
        );
    }

    #[test]
    fn soniox_cleans_up_on_success_poll_error_and_malformed_transcript() {
        for outcome in ["success", "poll-error", "malformed"] {
            let mut replies = vec![
                reply(200, r#"{"id":"file-1"}"#),
                reply(200, r#"{"id":"job-1"}"#),
            ];
            let mut expected = vec![
                "POST /files",
                "POST /transcriptions",
                "GET /transcriptions/job-1",
            ];
            if outcome == "poll-error" {
                replies.push(reply(
                    200,
                    r#"{"status":"error","error_message":"test-value"}"#,
                ));
            } else {
                replies.push(reply(200, r#"{"status":"completed"}"#));
                replies.push(reply(
                    200,
                    if outcome == "success" {
                        r#"{"tokens":[{"text":"Hello.","start_ms":0,"end_ms":1000}]}"#
                    } else {
                        r#"{"tokens":"test-value"}"#
                    },
                ));
                expected.push("GET /transcriptions/job-1/transcript");
            }
            replies.extend([reply(204, ""), reply(204, "")]);
            expected.extend(["DELETE /transcriptions/job-1", "DELETE /files/file-1"]);
            let (api, server) = server(replies);
            let audio = tempfile::NamedTempFile::new().unwrap();
            let request = Request {
                file: audio.path(),
                api_key: "test-value",
                model: None,
                language: None,
            };
            let result = soniox_with_client(&request, &mock_client(), &api);
            if outcome == "success" {
                assert_eq!(result.unwrap()[0].text, "Hello.");
            } else {
                assert!(!format!("{:#}", result.unwrap_err()).contains("test-value"));
            }
            assert_eq!(server.join().unwrap(), expected);
        }
    }

    #[test]
    fn failed_cleanup_is_reported_and_does_not_skip_the_other_resource() {
        let (api, server) = server(vec![reply(500, ""), reply(204, "")]);
        let warnings = cleanup_soniox(&mock_client(), &api, "test-value", "file-1", Some("job-1"));
        assert_eq!(warnings, ["could not delete Soniox transcriptions/job-1"]);
        assert_eq!(
            server.join().unwrap(),
            ["DELETE /transcriptions/job-1", "DELETE /files/file-1"]
        );
    }

    #[test]
    fn redirects_are_not_followed_and_production_requires_https() {
        let (api, server) = server(vec!["HTTP/1.1 307 Redirect\r\nLocation: /replayed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()]);
        let response = mock_client()
            .post(&api)
            .body("private audio")
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 307);
        assert_eq!(server.join().unwrap(), ["POST /"]);
        assert!(client().unwrap().post(&api).send().is_err());
    }

    #[test]
    fn oversized_responses_and_untrusted_ids_are_rejected() {
        let (api, server) = server(vec![format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_RESPONSE_BYTES + 1
        )]);
        let response = mock_client().get(&api).send().unwrap();
        assert!(
            read_json::<serde_json::Value>(response, "test", "test-value")
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
        server.join().unwrap();
        for id in ["", "../files", "x?query", "x#fragment", "x/y", "%2e%2e"] {
            assert!(resource_url("https://api.soniox.com/v1", "files", id).is_err());
        }
    }

    #[test]
    fn diagnostics_redact_keys_and_remove_terminal_controls() {
        let body =
            serde_json::json!({"message": "Incorrect key test-value\u{1b}[2J\n"}).to_string();
        let message = api_message(&body, "test-value");
        assert!(message.contains("[redacted]"));
        assert!(!message.contains("test-value"));
        assert!(!message.chars().any(char::is_control));
        assert_eq!(safe_message(&"x".repeat(1000), "").chars().count(), 301);
    }

    #[test]
    fn error_bodies_are_reduced_to_their_message() {
        assert_eq!(
            api_message(
                r#"{"error":{"message":"Invalid file format.","type":"x"}}"#,
                ""
            ),
            "Invalid file format."
        );
        assert_eq!(
            api_message(r#"{"error_message":"audio too short"}"#, ""),
            "audio too short"
        );
        assert_eq!(
            api_message("502 Bad Gateway", ""),
            "provider returned an error response"
        );
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
