use mime::Mime;
// This file is kind of overengineered in order to filter out silent parts manually.
use reqwest::header::HeaderMap;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

use std::env;
use tracing::error;

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

pub async fn transcribe(buffer: Vec<u8>, mime: Mime) -> Result<Option<String>, String> {
    // Set OpenAI API headers
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
        .mime_str(mime.to_string().as_str())
        .unwrap();
    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-large-v3")
        .text("response_format", "verbose_json")
        .part("file", part);

    // Send file to Groq Whisper for transcription
    let client = reqwest::Client::new();
    let res = client
        .post("https://api.groq.com/openai/v1/audio/transcriptions".to_string())
        .multipart(form)
        .headers(headers)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to OpenAI: {}", err);
            format!("Failed to send request to OpenAI: {}", err)
        })?;

    // IT'S EXTREMELY IMPORTANT TO HANDLE EVERY ERROR FROM HERE. WE CANNOT RETURN STATUS OTHER THEN 200, TELEGRAM IS GOING TO KEEP SENDING THE WEBHOOK AGAIN CREATING AN INFINITE LOOP.
    // Check if Groq returned an error
    let status = res.status();
    if !status.is_success() {
        let json = res
            .json::<serde_json::Value>()
            .await
            .map_err(|err| format!("Failed to parse OpenAI error response: {}", err))
            .unwrap();

        // Object {"error": Object {"code": String("rate_limit_exceeded"), "message": String("Rate limit reached for model `whisper-large-v3` in organization `org_01htnj6w5pf0za49my0yj0sje5` on seconds of audio per hour (ASPH): Limit 7200, Used 6816, Requested 607. Please try again in 1m51.463s. Visit https://console.groq.com/docs/rate-limits for more information."), "type": String("seconds")}}
        if json["error"]["code"] == "rate_limit_exceeded" {
            let wait_for = parse_groq_ratelimit_error(json["error"]["message"].as_str().unwrap()).await.unwrap();
            return Err(format!(
                "Rate limit reached. Please try again in {} seconds.",
                wait_for
            ));
        }

        error!("Groq returned an error: {:?}", json);
        return Err(format!("Groq returned an error: {}", json["error"]["message"]));
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
        return Ok(None);
    }

    Ok(Some(output_text))
}

pub async fn parse_groq_ratelimit_error(message: &str) -> Option<u32> {
    let re = regex::Regex::new(r"try again in (\d+)m(\d+\.?\d*)s").unwrap();
    let cap = re.captures(message)?;
    let minutes = cap[1].parse::<u32>().unwrap();
    let seconds = cap[2].parse::<f64>().unwrap().round() as u32;
    Some(minutes * 60 + seconds)
}