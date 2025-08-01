use crate::BASE_URL;
use log::{error, warn};
use mime::Mime;
use reqwest::header::AUTHORIZATION;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(strum::Display)]
pub enum TaskType {
    #[strum(to_string = "transcribe")]
    Transcribe,
    #[strum(to_string = "translate")]
    Translate,
}

#[derive(Debug, Deserialize, Serialize)]
struct GroqWhisperResponse {
    task: String,
    language: String,
    duration: f64,
    text: String,
    segments: Vec<GroqWhisperSegment>,
}

#[derive(Debug, Deserialize, Serialize)]
struct GroqWhisperSegment {
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

pub async fn transcribe(
    task_type: &TaskType,
    buffer: Vec<u8>,
    mime: Mime,
) -> Result<Option<String>, String> {
    // Set Groq API headers
    let mut headers: HeaderMap = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!(
            "Bearer {}",
            env::var("GROQ_API_KEY").expect("GROQ_API_KEY not found")
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
        .text("model", "whisper-large-v3")
        .text("response_format", "verbose_json")
        .part("file", part);

    // Send file to Groq Whisper for transcription
    let client = reqwest::Client::new();
    let url_ending = match task_type {
        TaskType::Transcribe => "/audio/transcriptions",
        TaskType::Translate => "/audio/translations",
    };

    let res = client
        .post(format!("{BASE_URL}{url_ending}"))
        .multipart(form)
        .headers(headers)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to OpenAI: {err}");
            format!("Failed to send request to OpenAI: {err}")
        })?;

    // IT'S EXTREMELY IMPORTANT TO HANDLE EVERY ERROR FROM HERE. WE CANNOT RETURN STATUS OTHER THEN 200, TELEGRAM IS GOING TO KEEP SENDING THE WEBHOOK AGAIN CREATING AN INFINITE LOOP.
    // Check if Groq returned an error
    let status = res.status();
    if !status.is_success() {
        let json = res
            .json::<serde_json::Value>()
            .await
            .map_err(|err| format!("Failed to parse OpenAI error response: {err}"));

        if let Err(err) = json {
            error!("{err}");
            return Err("Failed to parse OpenAI error response".to_string());
        }
        let json = json.unwrap();

        if json["error"]["code"] == "rate_limit_exceeded" {
            warn!("Rate limit reached. Here is the response: {json:?}");

            // DONT CHANGE THIS STRING!
            return Err("Rate limit reached.".to_string());
        }

        error!("Groq returned an error: {json:?}");
        return Err(format!("Groq returned an error: {}", json["error"]["code"]));
    }

    // Extract all of the segments
    let res = res.json::<GroqWhisperResponse>().await;

    if let Err(err) = res {
        error!("Failed to parse OpenAI response: {err}");
        return Err("Failed to parse OpenAI response".to_string());
    }

    let res = res.unwrap();

    let mut output_text = String::new();

    // Extract all of the segments.
    for segment in res.segments {
        // If the no_speech_prob value is higher than 1.0 and the avg_logprob is below -1, consider this segment silent.
        // These values are fine-tuned from a lot of testing. They work way better than the default values.
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
