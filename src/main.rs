use lambda_http::{run, service_fn, Body, Error, Request};
use mime::Mime;
use std::env;
use std::str::FromStr;
use teloxide::{net::Download, prelude::*};
use tracing::{debug, error, info, warn};
use tracing_subscriber::fmt;
use teloxide::types::Message;
use teloxide::types::UpdateKind;

mod transcribe;

const MAX_DURATION: u32 = 30 * 60;
const DEFAULT_DELAY: u64 = 5;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing for logging
    fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .without_time()
        .init();

    // Setup telegram bot (we do it here because this place is a cold start)
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set!"));

    // Set commands
    bot.set_my_commands(vec![])
        .await
        .expect("Failed to set commands");

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

    let audio_bytes: Vec<u8>;
    let mime;
    let duration;

    // Make sure the message is a voice or video note
    let message = match update.kind {
        UpdateKind::Message(message) => {
            if message.voice().is_none() && message.video_note().is_none() {
                debug!("Received non-voice, non-video note message");
                return Ok(lambda_http::Response::builder()
                    .status(200)
                    .body(String::new())
                    .unwrap());
            }
            message
        }
        _ => {
            debug!("Received non-message update");
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
    };

    // Send "typing" indicator
    debug!("Sending typing indicator");
    bot.send_chat_action(message.chat.id, teloxide::types::ChatAction::Typing)
        .await
        .unwrap();

    // Check if the message is a voice or video note
    if let Some(voice) = message.voice() {
        let filemeta = &voice.file;
        info!("Received voice message: {:?}", filemeta);

        (audio_bytes, mime, duration) = download_audio(&bot, &message).await?;

    } else if let Some(video_note) = message.video_note() {
        let filemeta = &video_note.file;
        info!("Received video note message: {:?}", filemeta);
        
        (audio_bytes, mime, duration) = download_audio(&bot, &message).await?;
    } else {
        debug!("Received non-voice, non-audio, non-video note message");
        return Ok(lambda_http::Response::builder()
            .status(200)
            .body(String::new())
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
        .disable_notification(true)
        .await
        .unwrap();

        return Ok(lambda_http::Response::builder()
            .status(200)
            .body(String::new())
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
            let bot_msg = bot
                .send_message(
                    message.chat.id,
                    format!("Failed to transcribe audio: {e:?}"),
                )
                .disable_web_page_preview(true)
                .disable_notification(true)
                .reply_to_message_id(message.id)
                .await
                .unwrap();

            delete_message_delay(&bot, &bot_msg, DEFAULT_DELAY).await;
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
    };

    // If the transcription is empty
    if transcription.is_none() {
        warn!("Transcription is empty!");
        let bot_msg = bot
            .send_message(message.chat.id, "<no text>")
            .reply_to_message_id(message.id)
            .disable_web_page_preview(true)
            .disable_notification(true)
            .await
            .unwrap();

        delete_message_delay(&bot, &bot_msg, DEFAULT_DELAY).await;

        return Ok(lambda_http::Response::builder()
            .status(200)
            .body(String::new())
            .unwrap());
    }
    let transcription = transcription.unwrap();

    // Send the transcription to the user
    info!("Transcription: {}", transcription);
    bot.send_message(message.chat.id, transcription.clone())
        .reply_to_message_id(message.id)
        .disable_web_page_preview(true)
        .disable_notification(true)
        .await
        .unwrap();

    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

async fn download_audio(bot: &Bot, message: &Message) -> Result<(Vec<u8>, Mime, u32), Error> {
    let mut audio_bytes = Vec::new();
    let mime;
    let duration;

    if let Some(audio) = message.audio() {
        let file = bot.get_file(&audio.file.id).await?;
        mime = audio.mime_type.clone().unwrap();
        duration = audio.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else if let Some(voice) = message.voice() {
        let file = bot.get_file(&voice.file.id).await?;
        mime = voice.mime_type.clone().unwrap_or_else(|| Mime::from_str("audio/ogg").unwrap());
        duration = voice.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else if let Some(video_note) = message.video_note() {
        let file = bot.get_file(&video_note.file.id).await?;
        mime = Mime::from_str("video/mp4").unwrap();
        duration = video_note.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else {
        return Err(Error::from("Unsupported message type"));
    }

    Ok((audio_bytes, mime, duration))
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

pub async fn delete_message_delay(bot: &Bot, msg: &Message, delay: u64) {
    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
    bot.delete_message(msg.chat.id, msg.id)
        .await
        .unwrap();
}


pub async fn parse_groq_ratelimit_error(message: &str) -> Option<u32> {
    let re = regex::Regex::new(r"try again in (\d+)m(\d+\.?\d*)s").unwrap();
    let cap = re.captures(message)?;
    let minutes = cap[1].parse::<u32>().unwrap();
    let seconds = cap[2].parse::<f64>().unwrap().round() as u32;
    Some(minutes * 60 + seconds)
}
