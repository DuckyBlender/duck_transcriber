use crate::types::TranscriptionError;
use log::{error, info};
use mime::Mime;
use serde::Deserialize;
use std::env;

pub const WHISPER_MODEL: &str = "whisper.cpp large-v3-turbo";
const DEFAULT_WHISPER_URL: &str = "http://host.docker.internal:8080/inference";
// How much of an unparseable response body to include in the log.
const MAX_LOGGED_BODY_CHARS: usize = 2000;

/// Transcribes audio with the local whisper.cpp server. Returns `None` when
/// the audio has no speech.
pub async fn transcribe(buffer: Vec<u8>, mime: Mime) -> Result<Option<String>, TranscriptionError> {
    let url = env::var("WHISPER_LOCAL_URL").unwrap_or_else(|_| DEFAULT_WHISPER_URL.to_string());
    info!("Transcribing with {WHISPER_MODEL} at {url}");

    // whisper-server --convert hands the upload to ffmpeg, which detects the
    // format from the content, so the file name only needs to look plausible.
    let part = reqwest::multipart::Part::bytes(buffer)
        .file_name(format!("audio.{}", mime.subtype()))
        .mime_str(mime.essence_str())
        .map_err(|e| {
            error!("Failed to parse MIME type for whisper.cpp: {e}");
            TranscriptionError::Parse("Invalid MIME type".to_string())
        })?;

    let form = reqwest::multipart::Form::new()
        .text("response_format", "verbose_json")
        .text("temperature", "0.0")
        .text("temperature_inc", "0.2")
        .text("language", "auto")
        .text("no_language_probabilities", "true")
        .part("file", part);

    let res = reqwest::Client::new()
        .post(url)
        .multipart(form)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to whisper.cpp: {err}");
            TranscriptionError::Network(format!("Failed to send request to whisper.cpp: {err}"))
        })?;

    let status = res.status();
    let body = res.text().await.map_err(|err| {
        error!("Failed to read whisper.cpp response body ({status}): {err}");
        TranscriptionError::Network(format!("Failed to read whisper.cpp response body: {err}"))
    })?;

    if !status.is_success() {
        error!("whisper.cpp returned {status}: {}", truncate_for_log(&body));
        return Err(TranscriptionError::Api(format!(
            "whisper.cpp error: {status}"
        )));
    }

    let res = parse_whisper_response(&body)?;
    let output_text = whisper_output_text(res);

    if output_text.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(output_text))
}

fn parse_whisper_response(body: &str) -> Result<WhisperResponse, TranscriptionError> {
    serde_json::from_str(body).map_err(|err| {
        error!(
            "Failed to parse whisper.cpp response: {err}; body: {}",
            truncate_for_log(body)
        );
        TranscriptionError::Parse("Failed to parse whisper.cpp response".to_string())
    })
}

fn whisper_output_text(res: WhisperResponse) -> String {
    if res.segments.is_empty() {
        return res.text;
    }

    res.segments
        .into_iter()
        .filter(|segment| {
            !is_silent_segment(
                segment.no_speech_prob.unwrap_or(0.0),
                segment.avg_logprob.unwrap_or(0.0),
            )
        })
        .map(|segment| segment.text)
        .collect()
}

pub fn truncate_for_log(body: &str) -> String {
    let mut chars = body.chars();
    let mut truncated: String = chars.by_ref().take(MAX_LOGGED_BODY_CHARS).collect();
    if chars.next().is_some() {
        truncated.push_str("… (truncated)");
    }
    truncated
}

fn is_silent_segment(no_speech_prob: f64, avg_logprob: f64) -> bool {
    // Tuned from production testing to reduce hallucinated silence.
    no_speech_prob > 0.6 && avg_logprob < -0.4
}

/// whisper.cpp verbose_json response. Only the fields the bot uses are read,
/// and all of them are optional so a missing field doesn't throw away an
/// otherwise usable transcription.
#[derive(Debug, Deserialize)]
struct WhisperResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    segments: Vec<WhisperSegment>,
}

#[derive(Debug, Deserialize)]
struct WhisperSegment {
    #[serde(default)]
    text: String,
    avg_logprob: Option<f64>,
    no_speech_prob: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_response_with_missing_fields() {
        let res = parse_whisper_response(
            r#"{"text":" whole","segments":[{"text":" Hello"},{"text":" world","avg_logprob":-0.1,"no_speech_prob":0.01}]}"#,
        )
        .unwrap();
        assert_eq!(whisper_output_text(res), " Hello world");
    }

    #[test]
    fn falls_back_to_text_without_segments() {
        let res = parse_whisper_response(r#"{"text":" Hello"}"#).unwrap();
        assert_eq!(whisper_output_text(res), " Hello");
    }

    #[test]
    fn drops_silent_segments() {
        let res = parse_whisper_response(
            r#"{"text":"","segments":[{"text":" Thanks for watching","avg_logprob":-0.9,"no_speech_prob":0.9},{"text":" Hi"}]}"#,
        )
        .unwrap();
        assert_eq!(whisper_output_text(res), " Hi");
    }

    #[test]
    fn rejects_non_json_body() {
        assert!(parse_whisper_response("<html>502</html>").is_err());
    }

    #[test]
    fn truncates_long_bodies_on_char_boundaries() {
        let body = "ż".repeat(MAX_LOGGED_BODY_CHARS + 5);
        let logged = truncate_for_log(&body);
        assert!(logged.ends_with("… (truncated)"));
        assert_eq!(
            logged.chars().filter(|c| *c == 'ż').count(),
            MAX_LOGGED_BODY_CHARS
        );
        assert_eq!(truncate_for_log("short"), "short");
    }
}
