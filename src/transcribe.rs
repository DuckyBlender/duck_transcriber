// This file is kind of overengineered in order to filter out silent parts manually.

use mime::Mime;
use reqwest::header::HeaderMap;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

use std::env;
use tracing::error;

// https://platform.openai.com/docs/guides/error-codes/api-errors
#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum OpenAIError {
    // short names
    InvalidAuth,
    IncorrectAPIKey,
    NotInOrg,
    RateLimit,
    QuotaExceeded,
    ServerError,
    Overloaded,
    Other(String),
}

#[derive(Debug, Deserialize, Serialize)]
struct OpenAIWhisperResponse {
    task: String,
    language: String,
    duration: f64,
    text: String,
    segments: Vec<OpenAIWhisperSegment>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OpenAIWhisperSegment {
    id: u32,
    seek: u32,
    start: f64,
    end: f64,
    text: String,
    tokens: Vec<u32>,
    temperature: f64,
    avg_logprob: f64,
    compression_ratio: f64,
    no_speech_prob: f64,
}

impl std::fmt::Display for OpenAIError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            OpenAIError::InvalidAuth => write!(f, "Invalid authentication"),
            OpenAIError::IncorrectAPIKey => write!(f, "Incorrect API key"),
            OpenAIError::NotInOrg => write!(f, "Not in an organization"),
            OpenAIError::RateLimit => write!(f, "Rate limit exceeded or quota exceeded"),
            OpenAIError::QuotaExceeded => write!(f, "Quota exceeded or rate limited"),
            OpenAIError::ServerError => write!(f, "Server error"),
            OpenAIError::Overloaded => write!(f, "Server overloaded"),
            OpenAIError::Other(err) => write!(f, "{}", err),
        }
    }
}

pub async fn transcribe(buffer: Vec<u8>, mime: Mime) -> Result<String, OpenAIError> {
    // Set OpenAI API headers
    let mut headers: HeaderMap = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!(
            "Bearer {}",
            env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not found")
        )
        .parse()
        .unwrap(),
    );

    // Create multipart request
    let part = reqwest::multipart::Part::bytes(buffer)
        .file_name(format!("audio.{}", mime.subtype()))
        .mime_str(mime.as_ref())
        .unwrap();
    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("response_format", "verbose_json")
        .part("file", part);

    // Send file to OpenAI Whisper for transcription
    let client = reqwest::Client::new();
    let res = client
        .post("https://api.openai.com/v1/audio/transcriptions".to_string())
        .multipart(form)
        .headers(headers)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to OpenAI: {}", err);
            OpenAIError::Other(format!("Failed to send request to OpenAI: {}", err))
        })?;

    // IT'S EXTREMELY IMPORTANT TO HANDLE EVERY ERROR FROM HERE. WE CANNOT RETURN STATUS OTHER THEN 200, TELEGRAM IS GOING TO KEEP SENDING THE WEBHOOK AGAIN CREATING AN INFINITE LOOP.
    // Check if OpenAI returned an error
    let status = res.status();
    if !status.is_success() {
        if status.as_u16() == 429 {
            // quota exceeded or rate limited (unlikely)
            return Err(OpenAIError::QuotaExceeded);
        }
        let json = res
            .json::<serde_json::Value>()
            .await
            .map_err(|err| format!("Failed to parse OpenAI error response: {}", err))
            .unwrap();
        error!("OpenAI returned an error: {:?}", json);
        return Err(OpenAIError::Other(format!(
            "OpenAI returned an error: {:?}",
            json
        )));
    }

    // Extract all of the segments
    let res = res
        .json::<OpenAIWhisperResponse>()
        .await
        .map_err(|err| format!("Failed to parse OpenAI response: {}", err))
        .unwrap();

    let mut output_text = String::new();

    // Extract all of the segments.
    for segment in res.segments {
        // If the no_speech_prob value is higher than 1.0 and the avg_logprob is below -1, consider this segment silent.
        if segment.no_speech_prob > 0.6 && segment.avg_logprob < -0.4 {
            continue;
        }
        output_text += &segment.text;
    }

    // If the output text is empty, return <no text>
    if output_text.is_empty() {
        return Ok("<no text>".to_string());
    }

    Ok(output_text)
}
