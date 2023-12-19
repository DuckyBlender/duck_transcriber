use lambda_http::{http::HeaderMap, Error};
use reqwest::header::AUTHORIZATION;
use std::env;

pub async fn transcribe_audio(buffer: Vec<u8>) -> Result<String, Error> {
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
        .file_name("audio.ogg")
        .mime_str("audio/ogg")
        .unwrap();
    let form = reqwest::multipart::Form::new()
        .text("model", "whisper-1")
        .text("response_format", "text")
        .part("file", part);

    // Send file to OpenAI Whisper for transcription
    let client = reqwest::Client::new();
    let res = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .multipart(form)
        .headers(headers)
        .send()
        .await?;

    // Extract text from OpenAI response
    let text = res.text().await?;

    Ok(text)
}
