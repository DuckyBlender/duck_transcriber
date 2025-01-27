use crate::BASE_URL;
use mime::Mime;
use reqwest::header::HeaderMap;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use std::env;
use tracing::error;
use tracing::warn;

#[derive(strum::Display)]
pub enum TaskType {
    #[strum(to_string = "transcribe")]
    Transcribe,
    #[strum(to_string = "translate")]
    Translate,
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

#[derive(Debug, Serialize)]
struct GroqChatRequest {
    model: String,
    messages: Vec<GroqChatMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Debug, Serialize)]
struct GroqChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct GroqChatResponse {
    choices: Vec<GroqChatChoice>,
}

#[derive(Debug, Deserialize)]
struct GroqChatChoice {
    message: GroqChatMessage,
}

pub async fn summarize(text: &str) -> Result<String, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!(
            "Bearer {}",
            env::var("GROQ_API_KEY").expect("GROQ_API_KEY not found")
        )
        .parse()
        .unwrap(),
    );

    let prompt = format!("Please summarize the following text in its original language, keeping the key points and main ideas. Make the summary concise but comprehensive:\n\n{}", text);

    let request = GroqChatRequest {
        model: "llama-3-8b-8192".to_string(),
        messages: vec![GroqChatMessage {
            role: "user".to_string(),
            content: prompt,
        }],
        temperature: 0.7,
        max_tokens: 512,
    };

    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/chat/completions", BASE_URL))
        .headers(headers)
        .json(&request)
        .send()
        .await
        .map_err(|err| format!("Failed to send request to Groq: {}", err))?;

    if !res.status().is_success() {
        let json = res
            .json::<serde_json::Value>()
            .await
            .map_err(|err| format!("Failed to parse Groq error response: {err}"))?;
        return Err(format!("Groq returned an error: {}", json["error"]["message"]));
    }

    let response = res
        .json::<GroqChatResponse>()
        .await
        .map_err(|err| format!("Failed to parse Groq response: {}", err))?;

    Ok(response.choices[0].message.content.trim().to_string())
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
            error!("Failed to send request to OpenAI: {}", err);
            format!("Failed to send request to OpenAI: {err}")
        })?;

    // IT'S EXTREMELY IMPORTANT TO HANDLE EVERY ERROR FROM HERE. WE CANNOT RETURN STATUS OTHER THEN 200, TELEGRAM IS GOING TO KEEP SENDING THE WEBHOOK AGAIN CREATING AN INFINITE LOOP.
    // Check if Groq returned an error
    let status = res.status();
    if !status.is_success() {
        let json = res
            .json::<serde_json::Value>()
            .await
            .map_err(|err| format!("Failed to parse OpenAI error response: {err}"))
            .unwrap();

        if json["error"]["code"] == "rate_limit_exceeded" {
            warn!("Rate limit reached. Here is the response: {:?}", json);

            // DONT CHANGE THIS STRING!
            return Err("Rate limit reached.".to_string());
        }

        error!("Groq returned an error: {:?}", json);
        return Err(format!("Groq returned an error: {}", json["error"]["code"]));
    }

    // Extract all of the segments
    let res = res.json::<OpenAIWhisperResponse>().await;

    if let Err(err) = res {
        error!("Failed to parse OpenAI response: {}", err);
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
