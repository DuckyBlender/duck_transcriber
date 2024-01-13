use lambda_http::{Body, Error, Request, Response};
use mime::Mime;

use crate::openai;
use crate::utils;
use std::env;
use teloxide::types::ChatAction::Typing;
use teloxide::{
    net::Download, payloads::SendMessageSetters, requests::Requester, types::UpdateKind, Bot,
};
use tracing::info;

const MINUTE_LIMIT: u32 = 5;
// const TELEGRAM_OWNER_ID: u64 = 5337682436;

#[derive(PartialEq)]
enum MediaType {
    Voice,
    VideoNote,
}

pub async fn handle_telegram_request(req: Request) -> Result<Response<Body>, Error> {
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    let update = utils::convert_input_to_json(req).await.unwrap();

    // Match the update type
    match update.kind {
        // If the update is a message
        UpdateKind::Message(message) => {
            // Check if the message is a voice or video message
            if message.voice().is_none() && message.video_note().is_none() {
                info!("Not a voice or video message");
                // don't send a message to the user
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::Text("Not a voice or video message".into()))
                    .unwrap());
            }

            let media_type = if message.voice().is_some() {
                info!("Received voice message");
                MediaType::Voice
            } else if message.video_note().is_some() {
                info!("Received video message");
                MediaType::VideoNote
            } else {
                info!("Message is not a voice or video message");
                // don't send a message to the user
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::Text("Message is not a voice or video message".into()))
                    .unwrap());
            };

            // Get the voice duration
            let duration = if let Some(voice) = message.voice() {
                voice.duration
            } else if let Some(video_note) = message.video_note() {
                video_note.duration
            } else {
                info!("Message is not a voice or video message");
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::Text("Message is not a voice or video message".into()))
                    .unwrap());
            };

            // Check if voice message is longer than 1 minute
            if duration > MINUTE_LIMIT * 60 {
                // Send a message to the user
                bot.send_message(
                    message.chat.id,
                    format!(
            "The audio message is too long. Maximum duration is {MINUTE_LIMIT} minutes."
        ),
                )
                .reply_to_message_id(message.id)
                .await?;
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::Text("Message too long".into()))
                    .unwrap());
            }

            // Now that we know that the voice message is shorter then x minutes, download it and send it to openai
            // Send "typing" action to user
            bot.send_chat_action(message.chat.id, Typing).await?;

            let voice_id = if media_type == MediaType::Voice {
                message.voice().unwrap().file.id.clone()
            } else {
                message.video_note().unwrap().file.id.clone()
            };

            // Get the voice mime type
            let default_mime: Mime = "audio/ogg".parse().unwrap();
            let voice_type: Mime = match message.voice() {
                Some(voice) => {
                    let voice_type = voice.mime_type.clone().unwrap_or(default_mime);
                    info!("Voice mime type: {}", voice_type.to_string().to_lowercase());
                    voice_type
                }
                None => default_mime,
            };

            let file = bot.get_file(voice_id).await?;
            let file_path = file.path.clone();
            let mut buffer = Vec::new();
            info!("Downloading file to buffer");
            bot.download_file(&file_path, &mut buffer).await?;

            // Send file to OpenAI Whisper for transcription
            let mut text = match openai::transcribe_audio(buffer, voice_type).await {
                Ok(text) => text,
                Err(e) => {
                    info!("Failed to transcribe audio: {}", e);
                    bot.send_message(
                        message.chat.id,
                        format!("Failed to transcribe audio. Please try again later. ({e})"),
                    )
                    .reply_to_message_id(message.id)
                    .await?;
                    return Ok(Response::builder()
                        .status(200)
                        .body(Body::Text(format!("Failed to transcribe audio: {e}")))
                        .unwrap());
                }
            };

            if text.is_empty() {
                text = "<no text>".to_string();
            }

            // Send text to user
            if let Err(e) = bot
                .send_message(message.chat.id, text)
                .reply_to_message_id(message.id)
                .disable_web_page_preview(true)
                .disable_notification(true)
                .allow_sending_without_reply(true)
                .await
            {
                info!("Failed to send message: {}", e);
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::Text("Failed to send message".into()))
                    .unwrap());
            }

            Ok(Response::builder()
                .status(200)
                .body(Body::Text("OK".into()))
                .unwrap())
        }
        // If the update is not a message
        _ => {
            info!("Update is not a message");
            Ok(Response::builder()
                .status(200)
                .body(Body::Text("Update is not a message".into()))
                .unwrap())
        }
    }
}
