use crate::BASE_URL;
use crate::types::{GroqWhisperResponse, TaskType, TranscriptionError};
use crate::utils;
use log::{error, info, warn};
use mime::Mime;
use reqwest::header::AUTHORIZATION;
use reqwest::header::HeaderMap;

pub async fn transcribe(
    task_type: &TaskType,
    buffer: Vec<u8>,
    mime: Mime,
) -> Result<Option<String>, TranscriptionError> {
    let api_keys = utils::get_api_keys();

    if api_keys.is_empty() {
        error!("No API keys configured");
        return Err(TranscriptionError::ApiError(
            "API key not configured".to_string(),
        ));
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
            Err(TranscriptionError::RateLimitReached) => {
                warn!(
                    "Rate limit reached with key {}, trying next key",
                    attempt + 1
                );
                last_error = Some(TranscriptionError::RateLimitReached);
                continue;
            }
            Err(e) => {
                error!("Error with key {}: {}", attempt + 1, e);
                last_error = Some(e);
                break;
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| TranscriptionError::ApiError("All API keys failed".to_string())))
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
    let part = reqwest::multipart::Part::bytes(buffer)
        .file_name(format!("audio.{}", mime.subtype()))
        .mime_str(mime.as_ref())
        .map_err(|e| {
            error!("Failed to parse MIME type: {e}");
            TranscriptionError::ParseError("Invalid MIME type".to_string())
        })?;
    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-large-v3")
        .text("response_format", "verbose_json")
        .part("file", part);

    // Send file to Groq Whisper for transcription
    let client = reqwest::Client::new();
    let url_ending = match task_type {
        TaskType::Transcribe => "/audio/transcriptions",
        TaskType::Translate => "/audio/translations",
        TaskType::Summarize => unreachable!("Summarize should not use Whisper API"),
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

    // Check if Groq returned an error
    let status = res.status();
    if !status.is_success() {
        let json = res.json::<serde_json::Value>().await.map_err(|err| {
            error!("Failed to parse OpenAI error response: {err}");
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

    // Extract all of the segments
    let res = res.json::<GroqWhisperResponse>().await.map_err(|err| {
        error!("Failed to parse OpenAI response: {err}");
        TranscriptionError::ParseError("Failed to parse API response".to_string())
    })?;

    let mut output_text = String::new();

    // Extract all of the segments.
    for segment in res.segments {
        // If the no_speech_prob value is higher than 1.0 and the avg_logprob is below -1, consider this segment silent.
        // These values are fine-tuned from a lot of testing. They work way better than the default values. No values are perfect, and there are still some hallucinations.
        if segment.no_speech_prob > 0.6 && segment.avg_logprob < -0.4 {
            continue;
        }
        output_text += &segment.text;
    }

    // If the output text is empty, return <no text>
    if output_text.is_empty() {
        return Ok(None);
    }

    Ok(Some(output_text))
}
