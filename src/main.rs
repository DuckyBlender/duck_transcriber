use lambda_http::{run, service_fn, Body, Error, Request};
use mime::Mime;
use std::env;
use std::str::FromStr;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::UpdateKind::Message;
use teloxide::{net::Download, requests::Requester, types::Update, Bot};
use tracing::{debug, error, info, warn};
use tracing_subscriber::fmt;

mod transcribe;

const MAX_DURATION: u32 = 10 * 60;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing for logging
    fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .without_time()
        .init();

    // Setup telegram bot (we do it here because this place is a cold start)
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set!"));

    // Run the Lambda function
    run(service_fn(|req| handler(req, &bot))).await
}

async fn handler(
    req: lambda_http::Request,
    bot: &Bot,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    // Parse JSON webhook
    let bot = bot.clone();
    let update = match parse_webhook(req).await {
        Ok(message) => message,
        Err(e) => {
            error!("Failed to parse webhook: {:?}", e);
            return Ok(lambda_http::Response::builder()
                .status(400)
                .body("Failed to parse webhook".into())
                .unwrap());
        }
    };

    // Make sure the message is a voice, audio or video note
    let message = match update.kind {
        Message(message) => {
            if message.voice().is_none()
                && message.audio().is_none()
                && message.video_note().is_none()
            {
                debug!("Received non-voice, non-audio, non-video note message");
                return Ok(lambda_http::Response::builder()
                    .status(200)
                    .body("".into())
                    .unwrap());
            }
            message
        }
        _ => {
            debug!("Received non-message update");
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body("".into())
                .unwrap());
        }
    };

    // Send "typing" indicator
    debug!("Sending typing indicator");
    bot.send_chat_action(message.chat.id, teloxide::types::ChatAction::Typing)
        .await
        .unwrap();

    let mut audio_bytes: Vec<u8> = Vec::new();
    let mime;
    let duration;

    // Check if the message is a voice, audio or video note
    if let Some(voice) = message.voice() {
        let file_id = &voice.file.id;
        let file = bot.get_file(file_id).await.unwrap();
        // default is ogg
        mime = voice
            .mime_type
            .clone()
            .unwrap_or(Mime::from_str("audio/ogg").unwrap());
        duration = voice.duration;
        bot.download_file(&file.path, &mut audio_bytes)
            .await
            .unwrap();
    // } else if let Some(audio) = message.audio() {
    //     let file_id = &audio.file.id;
    //     let file = bot.get_file(file_id).await.unwrap();
    //     mime = audio.mime_type.clone().unwrap_or(Mime::from_str("audio/ogg").unwrap());
    //     bot.download_file(&file.path, &mut audio_bytes)
    //         .await
    //         .unwrap();
    } else if let Some(video_note) = message.video_note() {
        let file_id = &video_note.file.id;
        let file = bot.get_file(file_id).await.unwrap();
        mime = Mime::from_str("video/mp4").unwrap();
        duration = video_note.duration;
        bot.download_file(&file.path, &mut audio_bytes)
            .await
            .unwrap();
    } else {
        debug!("Received non-voice, non-audio, non-video note message");
        return Ok(lambda_http::Response::builder()
            .status(200)
            .body("".into())
            .unwrap());
    }

    // If the duration is above MAX_DURATION
    if duration > MAX_DURATION {
        warn!("The audio message is above {MAX_DURATION} seconds!");
        bot.send_message(
            message.chat.id,
            format!("Duration is above {} minutes", MAX_DURATION * 60),
        )
        .reply_to_message_id(message.id)
        .await
        .unwrap();

        return Ok(lambda_http::Response::builder()
            .status(200)
            .body("".into())
            .unwrap());
    }

    // Transcribe the message
    info!(
        "Transcribing audio! Duration: {} | Mime: {:?}",
        duration, mime
    );
    let transcription = transcribe::transcribe(audio_bytes, mime).await;

    let transcription = match transcription {
        Ok(transcription) => transcription,
        Err(e) => {
            error!("Failed to transcribe audio: {:?}", e);
            bot.send_message(
                message.chat.id,
                format!("Failed to transcribe audio: {:?}", e),
            )
            .reply_to_message_id(message.id)
            .await
            .unwrap();
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body("".into())
                .unwrap());
        }
    };

    // Send the transcription to the user
    info!("Transcription: {}", transcription);
    bot.send_message(message.chat.id, transcription)
        .reply_to_message_id(message.id)
        .await
        .unwrap();

    Ok(lambda_http::Response::builder()
        .status(200)
        .body("".into())
        .unwrap())
}

pub async fn parse_webhook(input: Request) -> Result<Update, Error> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        not => panic!("expected Body::Text(...) got {not:?}"),
    };
    let body_json: Update = serde_json::from_str(body_str)?;
    debug!("Parsed webhook: {:?}", body_json);
    Ok(body_json)
}
