use lambda_http::{http::HeaderMap, run, service_fn, Body, Error, Request, Response};
use reqwest::header::AUTHORIZATION;
use serde_json::json;
use std::env;
use tracing::info;

const MINUTE_LIMIT: i64 = 5;

async fn handle_telegram_request(req: Request) -> Result<Response<Body>, Error> {
    // If the request is GET, return an empty response
    if req.method() == "GET" {
        return Ok(Response::builder()
            .status(200)
            .body("Hello from Rust!".into())
            .map_err(Box::new)?);
    }

    // Extract the request body
    let body = req.body();

    // Parse JSON body
    let json_body = serde_json::from_slice::<serde_json::Value>(body.as_ref())?;

    // Check if it's a voice message
    let is_voice_message = json_body["message"]["voice"].is_object();
    if !is_voice_message {
        // If not a voice message, log and return an empty response
        // TODO: Implement support for audio files (not just voice messages)
        info!("Not a voice message");
        return Ok(Response::builder()
            .status(200)
            .body(Body::Empty)
            .map_err(Box::new)?);
    }

    // Extract file_id from the voice message
    let file_id = json_body["message"]["voice"]["file_id"]
        .as_str()
        .ok_or_else(|| lambda_http::Error::from("Missing file_id in voice message"))?;

    // Check the duration of the audio file
    let duration = json_body["message"]["voice"]["duration"]
        .as_i64()
        .ok_or_else(|| lambda_http::Error::from("Missing duration in voice message"))?;
    if duration > MINUTE_LIMIT * 60 {
        // If the audio is longer than 5 minutes, send a "too long audio" message
        send_telegram_message(
            &json_body["message"]["chat"]["id"],
            format!(
                "The audio message is too long. Maximum duration is {} minutes.",
                MINUTE_LIMIT
            )
            .as_str(),
            Some(&json_body["message"]["message_id"]),
        )
        .await?;
        return Ok(Response::builder()
            .status(200)
            .body(Body::Empty)
            .map_err(Box::new)?);
    }

    // Send 'typing' action to Telegram
    send_telegram_action(&json_body["message"]["chat"]["id"], "typing").await?;

    // Fetch the file from Telegram and send it to OpenAI for transcription
    let text = fetch_and_transcribe(file_id).await?;

    // Send the transcribed text back to Telegram
    send_telegram_message(
        &json_body["message"]["chat"]["id"],
        &text,
        Some(&json_body["message"]["message_id"]),
    )
    .await?;

    Ok(Response::builder()
        .status(200)
        .body(Body::Empty)
        .map_err(Box::new)?)
}

async fn fetch_and_transcribe(file_id: &str) -> Result<String, Error> {
    info!("Fetching info about file from Telegram: {}", file_id);

    // Build Telegram file URL
    let file_url = format!(
        "https://api.telegram.org/bot{}/getFile?file_id={}",
        env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not found"),
        file_id
    );

    // Fetch file information from Telegram
    let file_info = reqwest::get(&file_url)
        .await?
        .json::<serde_json::Value>()
        .await?;

    // Extract file_path from Telegram response
    let file_path = file_info["result"]["file_path"]
        .as_str()
        .ok_or_else(|| lambda_http::Error::from("Missing file_path in Telegram response"))?;

    // Build URL to download the file from Telegram
    let file_url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not found"),
        file_path
    );

    info!("Fetching file from Telegram: {}", file_url);

    // Fetch the file from Telegram
    let file = reqwest::get(&file_url).await?.bytes().await?.to_vec();

    info!("Sending file to OpenAI Whisper for transcription");

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
    let part = reqwest::multipart::Part::bytes(file)
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

async fn send_telegram_message(
    chat_id: &serde_json::Value,
    text: &str,
    reply_to_message_id: Option<&serde_json::Value>,
) -> Result<(), Error> {
    info!("Sending message to Telegram: {}", text);
    // Build URL for sending text back to Telegram
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not found")
    );

    let text = format!(
        "{}\n<i>Powered by <a href=\"https://openai.com/research/whisper\">OpenAI Whisper</a></i>",
        text
    );

    // Build JSON body for sending text to Telegram
    let mut body = json!({
        "chat_id": chat_id,
        "text": text,
        "disable_notification": true,
        "disable_web_page_preview": true,
        "allow_sending_without_reply": true,
        "parse_mode": "HTML",
    });

    if let Some(reply_id) = reply_to_message_id {
        body["reply_to_message_id"] = reply_id.clone();
    }

    info!("Sending message to Telegram: {:?}", body);

    // Send text back to Telegram
    let client = reqwest::Client::new();
    let res = client
        .post(&url)
        // json() sets the Content-Type header to application/json
        .json(&body)
        .send()
        .await?;

    // Check if the response was successful
    if !res.status().is_success() {
        return Err(lambda_http::Error::from(format!(
            "Telegram responded with status code {}. Details: {:?}",
            res.status(),
            res
        )));
    }

    Ok(())
}

async fn send_telegram_action(chat_id: &serde_json::Value, action: &str) -> Result<(), Error> {
    info!("Sending action to Telegram: {}", action);
    // Build URL for sending action to Telegram
    let url = format!(
        "https://api.telegram.org/bot{}/sendChatAction",
        env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not found")
    );

    // Build JSON body for sending action to Telegram
    let body = json!({
        "chat_id": chat_id,
        "action": action,
    });

    // Send action to Telegram
    let client = reqwest::Client::new();
    let res = client
        .post(&url)
        // json() sets the Content-Type header to application/json
        .json(&body)
        .send()
        .await?;

    // Check if the response was successful
    if !res.status().is_success() {
        return Err(lambda_http::Error::from(format!(
            "Telegram responded with status code {}. Details: {:?}",
            res.status(),
            res
        )));
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing for logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .without_time()
        .init();

    // Run the Lambda function
    run(service_fn(handle_telegram_request)).await
}
