use lambda_http::{Body, Response};
use lambda_runtime::Error;
use teloxide::types::ChatAction;
use teloxide::{net::Download, payloads::SendMessageSetters, requests::Requester, Bot};
use tracing::{error, info};

use crate::utils::whisper::{transcribe_audio, TranscribeType};

pub async fn handle_english_command(
    bot: Bot,
    message: teloxide::types::Message,
) -> Result<Response<Body>, Error> {
    // WE NEED AN AUDIO INPUT HERE
    // USE THE AUDIO FROM THE REPLY
    if let Some(reply) = message.reply_to_message() {
        if let Some(voice) = reply.voice() {
            // Send typing indicator
            bot.send_chat_action(message.chat.id, ChatAction::Typing)
                .await?;

            // Get the file_id of the voice message
            let file_id = &voice.file.id;

            // Length of the voice message
            let duration = voice.duration;

            // Download the voice message
            let file = bot.get_file(file_id).await?;

            // Convert to bytes
            let file_path = file.path.clone();
            let mut buffer = Vec::new();
            info!("Downloading file to buffer");
            bot.download_file(&file_path, &mut buffer).await?;

            // Transcribe the voice message
            let translation = transcribe_audio(buffer, TranscribeType::Translate, duration).await;

            match translation {
                Ok(translation) => {
                    // Send the translation to the user
                    bot.send_message(message.chat.id, translation)
                        .reply_to_message_id(message.id)
                        .disable_web_page_preview(true)
                        .disable_notification(true)
                        .allow_sending_without_reply(true)
                        .await?;
                }
                Err(e) => {
                    error!("Failed to translate audio: {}", e);
                    bot.send_message(
                        message.chat.id,
                        format!("Failed to translate audio. Please try again later. ({e})"),
                    )
                    .reply_to_message_id(message.id)
                    .disable_web_page_preview(true)
                    .allow_sending_without_reply(true)
                    .await?;

                    return Ok(Response::builder()
                        .status(200)
                        .body(Body::Text(format!("Failed to translate audio: {e}")))
                        .unwrap());
                }
            }
        } else if let Some(video_note) = reply.video_note() {
            // Send typing indicator
            bot.send_chat_action(message.chat.id, ChatAction::Typing)
                .await?;

            // Get the file_id of the voice message
            let file_id = &video_note.file.id;

            // Length of the voice message
            let duration = video_note.duration;

            // Download the voice message
            let file = bot.get_file(file_id).await?;

            // Convert to bytes
            let file_path = file.path.clone();
            let mut buffer = Vec::new();
            info!("Downloading file to buffer");
            bot.download_file(&file_path, &mut buffer).await?;

            // Transcribe the voice message
            let translation = transcribe_audio(buffer, TranscribeType::Translate, duration).await;

            match translation {
                Ok(translation) => {
                    // Send the translation to the user
                    bot.send_message(message.chat.id, translation)
                        .reply_to_message_id(message.id)
                        .disable_web_page_preview(true)
                        .disable_web_page_preview(true)
                        .disable_notification(true)
                        .allow_sending_without_reply(true)
                        .await?;
                }
                Err(e) => {
                    error!("Failed to translate audio: {}", e);
                    bot.send_message(
                        message.chat.id,
                        format!("Failed to translate audio. Please try again later. ({e})"),
                    )
                    .reply_to_message_id(message.id)
                    .allow_sending_without_reply(true)
                    .disable_web_page_preview(true)
                    .await?;

                    return Ok(Response::builder()
                        .status(200)
                        .body(Body::Text(format!("Failed to translate audio: {e}")))
                        .unwrap());
                }
            }
        } else {
            bot.send_message(
                message.chat.id,
                "Please reply to a voice message with the /english command.",
            )
            .reply_to_message_id(message.id)
            .allow_sending_without_reply(true)
            .disable_web_page_preview(true)
            .await?;
        }
    } else {
        bot.send_message(
            message.chat.id,
            "Please reply to a voice message with the /english command.",
        )
        .reply_to_message_id(message.id)
        .allow_sending_without_reply(true)
        .disable_web_page_preview(true)
        .await?;
    }

    Ok(Response::builder()
        .status(200)
        .body(Body::Text("OK".into()))
        .unwrap())
}
