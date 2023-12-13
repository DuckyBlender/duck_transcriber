use lambda_http::{http::HeaderMap, run, service_fn, Body, Error, Request, Response};
use reqwest::header::AUTHORIZATION;
use std::env;
use tracing::info;

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

    // Build Telegram file URL
    let file_url = format!(
        "https://api.telegram.org/bot{}/getFile?file_id={}",
        env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not found"),
        file_id
    );

    // Fetch file information from Telegram
    let file_url = reqwest::Url::parse(&file_url)?;
    let file_info = reqwest::get(file_url)
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

    // Fetch the file from Telegram
    let file_url = reqwest::Url::parse(&file_url)?;
    let file = reqwest::get(file_url).await?.bytes().await?.to_vec();

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

    // Build URL for sending text back to Telegram
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not found")
    );

    // Build JSON body for sending text to Telegram
    let body = serde_json::json!({
        "chat_id": json_body["message"]["chat"]["id"],
        "text": text,
        "reply_to_message_id": json_body["message"]["message_id"],
    });

    // Send text back to Telegram
    let client = reqwest::Client::new();
    let res = client
        .post(&url)
        // json header is automatically set
        .json(&body)
        .send()
        .await?;

    // Check if the response was successful
    if !res.status().is_success() {
        return Err(lambda_http::Error::from(format!(
            "Telegram responded with status code {}. Details: {:?}",
            res.status(), res
        )));
    }

    Ok(Response::builder()
        .status(200)
        .body(Body::Empty)
        .map_err(Box::new)?)
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
