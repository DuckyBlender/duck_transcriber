use crate::BASE_URL;
use crate::types::{TaskType, TranscriptionError};
use crate::utils;
use log::{error, info, warn};
use mime::Mime;
use reqwest::header::AUTHORIZATION;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use std::env;

pub const TRANSCRIPTION_MODEL: &str = "whisper-large-v3-turbo";
pub const TRANSLATION_MODEL: &str = "whisper-large-v3";
pub const LOCAL_WHISPER_MODEL: &str = "large-v3-turbo";
const DEFAULT_LOCAL_WHISPER_URL: &str = "http://host.docker.internal:8080/inference";
// How much of an unparseable response body to include in the log.
const MAX_LOGGED_BODY_CHARS: usize = 2000;

pub async fn transcribe(
    task_type: &TaskType,
    buffer: Vec<u8>,
    mime: Mime,
    force_local_whisper: bool,
) -> Result<Option<String>, TranscriptionError> {
    if force_local_whisper {
        info!("Local whisper.cpp was forced for this request");
        return transcribe_with_local_whisper(task_type, buffer, mime).await;
    }

    let api_keys = utils::get_api_keys();

    if api_keys.is_empty() {
        warn!("No Groq API keys configured; using local whisper.cpp");
        return transcribe_with_local_whisper(task_type, buffer, mime).await;
    }

    // Try each API key until one succeeds
    let mut last_error = None;
    for (attempt, api_key) in api_keys.iter().enumerate() {
        info!(
            "Attempting transcription with API key {} of {}",
            attempt + 1,
            api_keys.len()
        );

        match transcribe_with_key(task_type, buffer.clone(), mime.clone(), api_key).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                warn!("Error with key {}: {}; trying next key", attempt + 1, e);
                last_error = Some(e);
                continue;
            }
        }
    }

    warn!(
        "All Groq API keys failed (last error: {:?}); falling back to local whisper.cpp",
        last_error
    );
    transcribe_with_local_whisper(task_type, buffer, mime).await
}

async fn transcribe_with_key(
    task_type: &TaskType,
    buffer: Vec<u8>,
    mime: Mime,
    api_key: &str,
) -> Result<Option<String>, TranscriptionError> {
    // Set Groq API headers
    let mut headers: HeaderMap = HeaderMap::new();

    let auth_value = format!("Bearer {}", api_key).parse().map_err(|e| {
        error!("Failed to parse authorization header: {e}");
        TranscriptionError::ParseError("Invalid API key format".to_string())
    })?;

    headers.insert(AUTHORIZATION, auth_value);

    // Create multipart request
    let part = audio_part(buffer, &mime).map_err(|e| {
        error!("Failed to parse MIME type: {e}");
        TranscriptionError::ParseError("Invalid MIME type".to_string())
    })?;
    let form = reqwest::multipart::Form::new()
        .text(
            "model",
            match task_type {
                TaskType::Translate => TRANSLATION_MODEL.to_string(),
                _ => TRANSCRIPTION_MODEL.to_string(),
            },
        )
        .text("response_format", "verbose_json")
        .part("file", part);

    // Send file to Groq Whisper for transcription
    let client = reqwest::Client::new();
    let url_ending = match task_type {
        TaskType::Transcribe => "/audio/transcriptions",
        TaskType::Translate => "/audio/translations",
        TaskType::Summarize | TaskType::Caveman => {
            unreachable!("Summarize/Caveman should not use Whisper API")
        }
    };

    let res = client
        .post(format!("{BASE_URL}{url_ending}"))
        .multipart(form)
        .headers(headers)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to OpenAI: {err}");
            TranscriptionError::NetworkError(format!("Failed to send request: {err}"))
        })?;

    let status = res.status();
    let body = res.text().await.map_err(|err| {
        error!("Failed to read Groq response body ({status}): {err}");
        TranscriptionError::NetworkError(format!("Failed to read response body: {err}"))
    })?;

    // Check if Groq returned an error
    if !status.is_success() {
        let json = serde_json::from_str::<serde_json::Value>(&body).map_err(|err| {
            error!(
                "Failed to parse Groq error response ({status}): {err}; body: {}",
                truncate_for_log(&body)
            );
            TranscriptionError::ParseError("Failed to parse API error response".to_string())
        })?;

        if json["error"]["code"] == "rate_limit_exceeded" {
            warn!("Rate limit reached. Here is the response: {json:?}");
            return Err(TranscriptionError::RateLimitReached);
        }

        error!("Groq returned an error: {json:?}");
        let error_code = json["error"]["code"].as_str().unwrap_or("unknown");
        return Err(TranscriptionError::ApiError(format!(
            "Groq error: {}",
            error_code
        )));
    }

    let res = parse_whisper_response(&body, "Groq")?;
    let output_text = whisper_output_text(res);

    // If the output text is empty, return <no text>
    if output_text.is_empty() {
        return Ok(None);
    }

    Ok(Some(output_text))
}

async fn transcribe_with_local_whisper(
    task_type: &TaskType,
    buffer: Vec<u8>,
    mime: Mime,
) -> Result<Option<String>, TranscriptionError> {
    let url =
        env::var("WHISPER_LOCAL_URL").unwrap_or_else(|_| DEFAULT_LOCAL_WHISPER_URL.to_string());
    info!(
        "Transcribing with local whisper.cpp model {} at {}",
        LOCAL_WHISPER_MODEL, url
    );

    let part = audio_part(buffer, &mime).map_err(|e| {
        error!("Failed to parse MIME type for local whisper.cpp: {e}");
        TranscriptionError::ParseError("Invalid MIME type".to_string())
    })?;

    let mut form = reqwest::multipart::Form::new()
        .text("response_format", "verbose_json")
        .text("temperature", "0.0")
        .text("temperature_inc", "0.2")
        .text("language", "auto")
        .text("no_language_probabilities", "true")
        .part("file", part);

    if matches!(task_type, TaskType::Translate) {
        form = form.text("translate", "true");
    }

    let res = reqwest::Client::new()
        .post(url)
        .multipart(form)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to local whisper.cpp: {err}");
            TranscriptionError::NetworkError(format!(
                "Failed to send request to local whisper.cpp: {err}"
            ))
        })?;

    let status = res.status();
    let body = res.text().await.map_err(|err| {
        error!("Failed to read local whisper.cpp response body ({status}): {err}");
        TranscriptionError::NetworkError(format!(
            "Failed to read local whisper.cpp response body: {err}"
        ))
    })?;

    if !status.is_success() {
        error!("Local whisper.cpp returned {status}: {body}");
        return Err(TranscriptionError::ApiError(format!(
            "Local whisper.cpp error: {status}"
        )));
    }

    let res = parse_whisper_response(&body, "local whisper.cpp")?;
    let output_text = whisper_output_text(res);

    if output_text.is_empty() {
        return Ok(None);
    }

    Ok(Some(output_text))
}

/// Builds the multipart file part. Groq accepts uploads by file extension
/// (flac mp3 mp4 mpeg mpga m4a ogg opus wav webm), so MIME aliases such as
/// `audio/vorbis` or `audio/x-m4a` are mapped to an accepted extension
/// instead of being uploaded as `audio.vorbis` / `audio.x-m4a`.
fn audio_part(buffer: Vec<u8>, mime: &Mime) -> reqwest::Result<reqwest::multipart::Part> {
    let (extension, content_type) = upload_format(mime);
    reqwest::multipart::Part::bytes(buffer)
        .file_name(format!("audio.{extension}"))
        .mime_str(&content_type)
}

fn upload_format(mime: &Mime) -> (String, String) {
    let known = match (mime.type_().as_str(), mime.subtype().as_str()) {
        ("audio", "ogg" | "vorbis" | "x-vorbis" | "x-vorbis+ogg") => Some(("ogg", "audio/ogg")),
        ("audio", "opus" | "x-opus+ogg") => Some(("opus", "audio/opus")),
        ("audio", "mpeg" | "mp3" | "mpeg3" | "x-mp3" | "x-mpeg" | "x-mpeg-3") => {
            Some(("mp3", "audio/mpeg"))
        }
        ("audio", "mp4") => Some(("mp4", "audio/mp4")),
        // Groq does not list .aac; its decoder sniffs the content, so send
        // raw AAC under an accepted extension.
        ("audio", "m4a" | "x-m4a" | "aac" | "x-aac" | "aacp") => Some(("m4a", "audio/m4a")),
        ("audio", "flac" | "x-flac") => Some(("flac", "audio/flac")),
        ("audio", "wav" | "x-wav" | "wave" | "vnd.wave") => Some(("wav", "audio/wav")),
        ("audio", "webm") => Some(("webm", "audio/webm")),
        ("video", "mp4" | "quicktime" | "x-m4v") => Some(("mp4", "video/mp4")),
        ("video", "webm") => Some(("webm", "video/webm")),
        _ => None,
    };

    match known {
        Some((extension, content_type)) => (extension.to_string(), content_type.to_string()),
        None => (mime.subtype().to_string(), mime.essence_str().to_string()),
    }
}

fn parse_whisper_response(body: &str, source: &str) -> Result<WhisperResponse, TranscriptionError> {
    serde_json::from_str(body).map_err(|err| {
        error!(
            "Failed to parse {source} response: {err}; body: {}",
            truncate_for_log(body)
        );
        TranscriptionError::ParseError(format!("Failed to parse {source} response"))
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

fn truncate_for_log(body: &str) -> String {
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

/// verbose_json response shared by Groq and whisper.cpp. Only the fields the
/// bot uses are read, and all of them are optional so a missing field doesn't
/// throw away an otherwise usable transcription.
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
    use std::str::FromStr;

    fn format_for(mime: &str) -> (String, String) {
        upload_format(&Mime::from_str(mime).unwrap())
    }

    #[test]
    fn maps_mime_aliases_to_groq_extensions() {
        let cases = [
            ("audio/vorbis", "ogg", "audio/ogg"),
            ("audio/x-vorbis+ogg", "ogg", "audio/ogg"),
            ("audio/ogg", "ogg", "audio/ogg"),
            ("audio/opus", "opus", "audio/opus"),
            ("audio/mpeg", "mp3", "audio/mpeg"),
            ("audio/x-m4a", "m4a", "audio/m4a"),
            ("audio/x-aac", "m4a", "audio/m4a"),
            ("audio/x-flac", "flac", "audio/flac"),
            ("audio/x-wav", "wav", "audio/wav"),
            ("video/quicktime", "mp4", "video/mp4"),
            ("video/mp4", "mp4", "video/mp4"),
        ];
        for (mime, extension, content_type) in cases {
            assert_eq!(
                format_for(mime),
                (extension.to_string(), content_type.to_string()),
                "{mime}"
            );
        }
    }

    #[test]
    fn unknown_mime_keeps_its_subtype() {
        assert_eq!(
            format_for("audio/amr"),
            ("amr".to_string(), "audio/amr".to_string())
        );
    }

    #[test]
    fn parses_response_with_missing_fields() {
        let res = parse_whisper_response(
            r#"{"text":" whole","segments":[{"text":" Hello"},{"text":" world","avg_logprob":-0.1,"no_speech_prob":0.01}]}"#,
            "test",
        )
        .unwrap();
        assert_eq!(whisper_output_text(res), " Hello world");
    }

    #[test]
    fn falls_back_to_text_without_segments() {
        let res = parse_whisper_response(r#"{"text":" Hello"}"#, "test").unwrap();
        assert_eq!(whisper_output_text(res), " Hello");
    }

    #[test]
    fn drops_silent_segments() {
        let res = parse_whisper_response(
            r#"{"text":"","segments":[{"text":" Thanks for watching","avg_logprob":-0.9,"no_speech_prob":0.9},{"text":" Hi"}]}"#,
            "test",
        )
        .unwrap();
        assert_eq!(whisper_output_text(res), " Hi");
    }

    #[test]
    fn rejects_non_json_body() {
        assert!(parse_whisper_response("<html>502</html>", "test").is_err());
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
