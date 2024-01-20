use mime::Mime;
use reqwest::header::HeaderMap;
use reqwest::header::AUTHORIZATION;
use std::env;
use tracing::error;

pub async fn transcribe_audio(buffer: Vec<u8>, voice_type: Mime) -> Result<String, String> {
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

    // Send file to OpenAI Whisper for transcription
    let client = reqwest::Client::new();
    let res = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .multipart(form)
        .headers(headers)
        .send()
        .await
        .map_err(|err| format!("Failed to send request to OpenAI: {}", err))?;

    // Check if OpenAI returned an error
    let status = res.status();
    if !status.is_success() {
        let json = res
            .json::<serde_json::Value>()
            .await
            .map_err(|err| format!("Failed to parse OpenAI error response: {}", err))?;
        error!("OpenAI returned an error: {:?}", json);
        return Err(format!("OpenAI returned an error: {:?}", json));
    }

    // Extract text from OpenAI response
    // Response is in verbose_json
    let json = res
        .json::<serde_json::Value>()
        .await
        .map_err(|err| format!("Failed to parse OpenAI response: {}", err))?;

    // Get the text from the response
    let text = match json["text"].as_str() {
        Some(text) => text.to_string(),
        None => "No text found".to_string(),
    };

    // Get the language from the response
    // let language = match json["language"].as_str() {
    //     Some(language) => language.to_string(),
    //     None => "No language found".to_string(),
    // };

    // Format the output
    // let output = format!("{}\n(Language: {})", text, language);

    Ok(text)
}
