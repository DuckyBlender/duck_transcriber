use lambda_http::{Body, Error, Request, Response};

use std::env;
use teloxide::{
    net::Download,
    payloads::SendMessageSetters,
    requests::Requester,
    types::UpdateKind,
    Bot,
};
use tracing::info;
use teloxide::types::ChatAction::Typing;

use crate::bedrock;
use crate::openai;
use crate::utils;

const MINUTE_LIMIT: u32 = 5;

#[derive(PartialEq)]
enum MediaType {
    Voice,
    VideoNote,
}

pub async fn handle_telegram_request(req: Request) -> Result<Response<Body>, Error> {
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    let update = utils::convert_input_to_json(req).await?;

    // Match the update type
    match update.kind {
        // If the update is a message
        UpdateKind::Message(message) => {
            // Check if the message is a text message
            if let Some(text) = message.text() {
                // Check if the text starts with /img
                if text.starts_with("/img") {
                    // Get the prompt
                    let prompt = text.replace("/img", "").trim().to_string();
                    // Send "typing" action to user
                    bot.send_chat_action(message.chat.id, Typing)
                        .await?;

                    

                    // Generate the image
                    let image = bedrock::generate_image(prompt).await.unwrap();
                    let image = teloxide::types::InputFile::memory(image);

                    // Send the image to the user
                    bot.send_photo(message.chat.id, image)
                        .await?;
                    return Ok(Response::builder()
                        .status(200)
                        .body(Body::Text("Image sent".into()))
                        .unwrap());
                }
            }
            // Check if the message is a voice message
            if message.voice().is_none() && message.video_note().is_none() {
                info!("Not a voice or video message");
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::Text("Not a voice or video message".into()))
                    .unwrap());
            }

            let media_type = if message.voice().is_some() {
                info!("Received voice message");
                MediaType::Voice
            } else {
                info!("Received video message");
                MediaType::VideoNote
            };

            // Get the voice duration 
            let duration = if message.voice().is_some() {
                message.voice().unwrap().duration
            } else {
                message.video_note().unwrap().duration
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
            bot.send_chat_action(message.chat.id, Typing)
                .await?;

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
            info!("Sending file to OpenAI Whisper for transcription");
            let mut text = openai::transcribe_audio(buffer).await?;

            if text.is_empty() {
                text = "<no text>".to_string();
            }

            // Send text to user
            bot.send_message(message.chat.id, text)
                .reply_to_message_id(message.id)
                .disable_web_page_preview(true)
                .disable_notification(true)
                .allow_sending_without_reply(true)
                .await?;
        }
        _ => {}
    }

    Ok(Response::builder()
        .status(200)
        .body(Body::Text("OK".into()))
        .unwrap())
}
