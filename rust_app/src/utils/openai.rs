use mime::Mime;
use reqwest::header::HeaderMap;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use tracing::error;

pub const MINUTE_LIMIT: usize = 30;

pub enum TranscribeType {
    Transcribe,
    Translate,
}

/// For the end part of the URL to OpenAI
impl std::fmt::Display for TranscribeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscribeType::Transcribe => write!(f, "transcriptions"),
            TranscribeType::Translate => write!(f, "translations"),
        }
    }
}

// dont warn about unused variants
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum Voice {
    Alloy,
    Echo,
    Fable,
    Onyx,
    Nova,
    Shimmer,
}

impl std::fmt::Display for Voice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Voice::Alloy => write!(f, "Alloy"),
            Voice::Echo => write!(f, "Echo"),
            Voice::Fable => write!(f, "Fable"),
            Voice::Onyx => write!(f, "Onyx"),
            Voice::Nova => write!(f, "Nova"),
            Voice::Shimmer => write!(f, "Shimmer"),
        }
    }
}

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

pub async fn transcribe_audio(
    buffer: Vec<u8>,
    voice_type: Mime,
    transcribe_type: TranscribeType,
    seconds: u32,
) -> Result<String, OpenAIError> {
    let seconds = seconds as usize;
    // Check if length of audio is more than x seconds
    if seconds > MINUTE_LIMIT * 60 {
        return Err(OpenAIError::Other(format!(
            "Audio length is more than {} minutes",
            MINUTE_LIMIT
        )));
    }

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

    let file_name = voice_type.subtype().to_string(); // "mpeg"
    let mime_str = voice_type.to_string(); // "audio/mpeg"

    // Create multipart request
    let part = reqwest::multipart::Part::bytes(buffer)
        .file_name(format!("audio.{}", file_name))
        .mime_str(mime_str.as_str())
        .unwrap();
    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("response_format", "verbose_json")
        .part("file", part);

    let url_end = transcribe_type.to_string();

    // Send file to OpenAI Whisper for transcription
    let client = reqwest::Client::new();
    let res = client
        .post(format!("https://api.openai.com/v1/audio/{}", url_end))
        .multipart(form)
        .headers(headers)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to OpenAI: {}", err);
            OpenAIError::Other(format!("Failed to send request to OpenAI: {}", err))
        })?;

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

// this returns audio bytes
pub async fn tts(prompt: String, voice: Voice) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::new();
    let res = client
        .post("https://api.openai.com/v1/audio/speech")
        .header(
            "Authorization",
            format!(
                "Bearer {}",
                env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not found")
            ),
        )
        .json(&json!({
            "model": "tts-1",
            "input": prompt,
            "voice": match voice {
                Voice::Alloy => "alloy",
                Voice::Echo => "echo",
                Voice::Fable => "fable",
                Voice::Onyx => "onyx",
                Voice::Nova => "nova",
                Voice::Shimmer => "shimmer",
            },
        }))
        .send()
        .await
        .map_err(|err| format!("Failed to send request to OpenAI: {}", err))?;

    // Check if OpenAI returned an error
    let status = res.status();
    if !status.is_success() {
        //    return an error
        return Err(format!("OpenAI returned an error: {:?}", status));
    }

    // Return the audio bytes
    let audio = res
        .bytes()
        .await
        .map_err(|err| format!("Failed to parse OpenAI response: {}", err))?;

    Ok(audio.to_vec())
}
