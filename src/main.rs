use lambda_http::{
    aws_lambda_events, http::HeaderMap, run, service_fn, Body, Error, Request, Response,
};
use reqwest::header::AUTHORIZATION;
use serde_json::json;
use std::env;
use teloxide::{
    net::Download,
    payloads::SendMessageSetters,
    requests::Requester,
    types::{ParseMode, Update, UpdateKind},
    Bot,
};
use tracing::info;

const MINUTE_LIMIT: u32 = 5;

async fn convert_input_to_json(input: Request) -> Result<Update, Error> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        not => panic!("expected Body::Text(...) got {:?}", not),
    };
    let body_json: Update = serde_json::from_str(body_str).unwrap();
    Ok(body_json)
}

async fn handle_telegram_request(req: Request) -> Result<Response<Body>, Error> {
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    let update = convert_input_to_json(req).await.unwrap();
    info!("update: {:?}", update);

    // Match the update type
    match update.kind {
        // If the update is a message
        UpdateKind::Message(message) => {
            // Check if the message is a voice message
            if let None = message.voice() {
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::from("Not a voice message"))
                    .unwrap());
            }

            let voice = message.voice().unwrap();

            // Check if voice message is longer than 1 minute
            if voice.duration > MINUTE_LIMIT {
                // Send a message to the user
                bot.send_message(
                    message.chat.id,
                    format!(
                        "The audio message is too long. Maximum duration is {} minutes.",
                        MINUTE_LIMIT
                    ),
                )
                .await
                .unwrap();
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::from("Voice message too long"))
                    .unwrap());
            }

            // Now that we know that the voice message is shorter then x minutes, download it and send it to openai
            let voice_id = voice.file.id.clone();
            let file = bot.get_file(voice_id).await.unwrap();
            // This object represents a file ready to be downloaded.
            // The file can be downloaded via the [Bot::download_file(file_path, dst)] method. It is guaranteed that the path from [GetFile] will be valid for at least 1 hour. When the path expires, a new one can be requested by calling [GetFile].

            let file_path = file.path.clone();
            let mut buffer = Vec::new();
            bot.download_file(&file_path, &mut buffer).await.unwrap();
            // Great, now we  have a Vec<u8> we can pass to reqwest and to OpenAI

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

            let text = format!(
                "{}\n<i>Powered by <a href=\"https://openai.com/research/whisper\">OpenAI Whisper</a></i>",
                text
            );

            // Send text to user
            bot.send_message(message.chat.id, text)
                .reply_to_message_id(message.id)
                .parse_mode(ParseMode::Html)
                .disable_web_page_preview(true)
                .disable_notification(true)
                .allow_sending_without_reply(true)
                .await
                .unwrap();
        }
        _ => {}
    }

    return Ok(Response::builder()
        .status(200)
        .body(Body::from("Hello, world!"))
        .unwrap());
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
