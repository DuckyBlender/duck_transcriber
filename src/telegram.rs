use lambda_http::{Body, Error, Request, Response};
use teloxide::payloads::SendPhotoSetters;
use teloxide::types::UserId;

use std::env;
use teloxide::types::ChatAction::Typing;
use teloxide::{
    net::Download, payloads::SendMessageSetters, requests::Requester, types::UpdateKind, Bot,
};
use tracing::info;

use crate::bedrock;
use crate::openai;
use crate::utils;

const MINUTE_LIMIT: u32 = 5;
const TELEGRAM_OWNER_ID: u64 = 5337682436;

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
            // Check if the message is a text message
            if let Some(text) = message.text() {
                // Check if the text starts with /img
                if text.starts_with("/img") {
                    // Check if the user is the owner
                    if message.from().unwrap().id != UserId(TELEGRAM_OWNER_ID) {
                        info!("User is not the owner");
                        return Ok(Response::builder()
                            .status(200)
                            .body(Body::Text("You are not the owner".into()))
                            .unwrap());
                    }

                    // Get the prompt
                    let prompt = text.replace("/img", "").trim().to_string();
                    // Send "typing" action to user
                    bot.send_chat_action(message.chat.id, Typing).await?;

                    // Also send a DM to the owner of the bot
                    // This is extremely temporary and is only used for preventing abuse.
                    // This will be removed in the near future.
                    // Actually this is kinda useless since only the owner can use this feature
                    let user = message.from().unwrap();
                    let _ = bot.send_message(
                        UserId(TELEGRAM_OWNER_ID),
                        format!(
                            "User {} ({} {}) requested an image with prompt: {}",
                            user.id,
                            user.first_name,
                            user.last_name.clone().unwrap_or_default(),
                            prompt
                        ),
                    );

                    // Generate the image
                    let image = match bedrock::generate_image(prompt).await {
                        Ok(image) => image,
                        Err(e) => {
                            info!("Failed to generate image: {}", e);
                            bot.send_message(
                                message.chat.id,
                                format!(
                                    "Failed to generate image. Please try again later. ({})",
                                    e
                                ),
                            )
                            .await
                            .unwrap();
                            return Ok(Response::builder()
                                .status(200)
                                .body(Body::Text(format!("Failed to generate image: {}", e)))
                                .unwrap());
                        }
                    };
                    let image = teloxide::types::InputFile::memory(image);

                    // Send the image to the user
                    if let Err(e) = bot
                        .send_photo(message.chat.id, image)
                        .reply_to_message_id(message.id)
                        .allow_sending_without_reply(true)
                        .await
                    {
                        info!("Failed to send image: {}", e);
                        bot.send_message(
                            message.chat.id,
                            format!("Failed to send image. Please try again later. ({})", e),
                        )
                        .await?;
                        return Ok(Response::builder()
                            .status(200)
                            .body(Body::Text(format!("Failed to send image: {}", e)))
                            .unwrap());
                    }

                    return Ok(Response::builder()
                        .status(200)
                        .body(Body::Text("Image sent".into()))
                        .unwrap());
                }
            }

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
            let file = bot.get_file(voice_id).await?;
            let file_path = file.path.clone();
            let mut buffer = Vec::new();
            info!("Downloading file to buffer");
            bot.download_file(&file_path, &mut buffer).await?;

            // Send file to OpenAI Whisper for transcription
            let mut text = match openai::transcribe_audio(buffer).await {
                Ok(text) => text,
                Err(e) => {
                    info!("Failed to transcribe audio: {}", e);
                    bot.send_message(
                        message.chat.id,
                        format!(
                            "Failed to transcribe audio. Please try again later. ({})",
                            e
                        ),
                    )
                    .await?;
                    return Ok(Response::builder()
                        .status(200)
                        .body(Body::Text(format!("Failed to transcribe audio: {}", e)))
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
