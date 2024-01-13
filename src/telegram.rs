use lambda_http::{Body, Error, Request, Response};
use mime::Mime;
use sqlx::{MySql, Pool};

use sqlx::Row;
use std::env;
use std::sync::Arc;
use teloxide::types::ChatAction::Typing;
use teloxide::types::ParseMode::MarkdownV2;
use teloxide::{
    net::Download, payloads::SendMessageSetters, requests::Requester, types::UpdateKind, Bot,
};
use tracing::info;

use crate::openai;
use crate::sql::handle_sql_command;
use crate::utils::{self, HELP_MESSAGE};
use crate::utils::{parse_argument, SqlCommands};

pub const MINUTE_LIMIT: u32 = 5;

#[derive(PartialEq)]
enum MediaType {
    Voice,
    VideoNote,
}

pub async fn handle_telegram_request(
    req: Request,
    pool: Arc<Pool<MySql>>,
) -> Result<Response<Body>, Error> {
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    let update = utils::convert_input_to_json(req).await.unwrap();

    // Match the update type
    match update.kind {
        // If the update is a message
        UpdateKind::Message(message) => {
            // Check if the message is a command
            // Available commands:
            // /help - Show help message
            // /debug - Enable or disable debug information
            // /remove - Enable or disable removing the original voice message after transcription
            // /gpt - Enable or disable enhancing the transcribed text with GPT (currently disabled)
            // Check if the message is a /sql command
            if let Some(text) = message.text() {
                let command = text.split_whitespace().next().unwrap_or("");
                info!("Received command: {}", command);
                match command {
                    // /help or /start
                    "/help" | "/start" => {
                        // Just send a message to the user
                        bot.send_message(message.chat.id, HELP_MESSAGE.to_string())
                            .parse_mode(MarkdownV2)
                            .reply_to_message_id(message.id)
                            .await?;
                    }
                    "/debug" => {
                        // Handle debug command
                        let state = parse_argument(text);
                        let debug_info = handle_sql_command(
                            &pool,
                            SqlCommands::DebugInfo(state, message.chat.id),
                        )
                        .await?;
                        // Send a message to the user
                        bot.send_message(
                            message.chat.id,
                            format!("Set debug_info to {}", debug_info),
                        )
                        .reply_to_message_id(message.id)
                        .await?;
                    }
                    "/remove" => {
                        // Handle remove command
                        let state = parse_argument(text);
                        let delete_voice = handle_sql_command(
                            &pool,
                            SqlCommands::RemoveOriginalVoice(state, message.chat.id),
                        )
                        .await?;
                        // Send a message to the user
                        bot.send_message(
                            message.chat.id,
                            format!("Set delete_voice to {}", delete_voice),
                        )
                        .reply_to_message_id(message.id)
                        .await?;
                    }
                    "/gpt_enhance" => {
                        // This command is currently disabled
                        bot.send_message(message.chat.id, "This command is currently disabled")
                            .reply_to_message_id(message.id)
                            .await?;
                    }
                    _ => {
                        // Do nothing
                        info!("Command not recognized");
                    }
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

// Define a new function to get the group settings
async fn get_group_settings(pool: &Pool<MySql>, group_id: i64) -> Result<(), sqlx::Error> {
    // Fetch the settings from the database using the group_id
    let settings = sqlx::query(
        "SELECT debug_info, delete_voice, gpt_enhance FROM transcriber WHERE chat_id = ?",
    )
    .bind(group_id)
    .fetch_one(pool)
    .await?;

    // Get the settings
    let debug_info: bool = settings.get("debug_info");
    let delete_voice: bool = settings.get("delete_voice");
    let gpt_enhance: bool = settings.get("gpt_enhance");

    // Return the settings
    todo!();
}
