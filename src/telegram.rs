use crate::openai::{self, TranscribeType, Voice};
use crate::openai::{transcribe_audio, tts};
use crate::utils;
use lambda_http::{Body, Error, Request, Response};
use mime::Mime;
use std::env;
use teloxide::payloads::SendVoiceSetters;
use teloxide::types::ChatAction::RecordVoice;
use teloxide::types::ChatAction::Typing;
use teloxide::types::InputFile;
use teloxide::{
    net::Download, payloads::SendMessageSetters, requests::Requester, types::UpdateKind, Bot,
};
use tracing::{error, info};

const MINUTE_LIMIT: u32 = 5;
// const TELEGRAM_OWNER_ID: u64 = 5337682436;

pub struct MessageInfo {
    pub is_text: bool,
    pub is_voice: bool,
    pub is_video_note: bool,
}

pub async fn handle_telegram_request(req: Request) -> Result<Response<Body>, Error> {
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    let update = utils::convert_input_to_json(req).await.unwrap();

    // Match the update type
    match update.kind {
        // If the update is a message
        UpdateKind::Message(message) => {
            // Get all the info about the message
            let message_info = MessageInfo {
                is_text: message.text().is_some(),
                is_voice: message.voice().is_some(),
                is_video_note: message.video_note().is_some(),
            };

            match message_info {
                MessageInfo { is_text: true, .. } => {
                    info!("Received text message");
                    // Get the text from the message
                    let text = message.text().unwrap();

                    if text.starts_with("/tts") || text.starts_with("/tts@duck_transcriber_bot") {
                        // CHECK FOR REPLY (WE NEED A TEXT INPUT)
                        // IF THERE IS NO REPLY, USE THE TEXT FROM THE COMMAND
                        let tts_text;
                        if let Some(reply) = message.reply_to_message() {
                            if let Some(text) = reply.text() {
                                tts_text = text;
                            } else {
                                tts_text = "";
                            }
                        } else {
                            // Get the text from the command
                            tts_text = text
                                .trim_start_matches("/tts")
                                .trim()
                                .trim_start_matches("/tts@duck_transcriber_bot")
                                .trim();
                        }

                        // If the text is empty, send a message to the user
                        if tts_text.is_empty() {
                            bot.send_message(
                                message.chat.id,
                                "Please provide some text to generate a voice message.",
                            )
                            .reply_to_message_id(message.id)
                            .await?;
                            return Ok(Response::builder()
                                .status(200)
                                .body(Body::Text("No text provided".into()))
                                .unwrap());
                        }

                        // Send "recording voice message" action to user
                        bot.send_chat_action(message.chat.id, RecordVoice).await?;

                        let voice = tts(tts_text.to_string(), Voice::Alloy).await;

                        match voice {
                            Ok(voice) => {
                                // Send the voice message to the user
                                bot.send_voice(message.chat.id, InputFile::memory(voice))
                                    .reply_to_message_id(message.id)
                                    .await?;
                            }
                            Err(e) => {
                                error!("Failed to generate voice: {}", e);
                                bot.send_message(
                                    message.chat.id,
                                    format!(
                                        "Failed to generate voice. Please try again later. ({e})"
                                    ),
                                )
                                .reply_to_message_id(message.id)
                                .await?;

                                return Ok(Response::builder()
                                    .status(200)
                                    .body(Body::Text(format!("Failed to generate voice: {e}")))
                                    .unwrap());
                            }
                        }
                    } else if text.starts_with("/english")
                        || text.starts_with("/english@duck_transcriber_bot")
                    {
                        // WE NEED AN AUDIO INPUT HERE
                        // USE THE AUDIO FROM THE REPLY
                        if let Some(reply) = message.reply_to_message() {
                            if let Some(voice) = reply.voice() {
                                // TODO: Check if this is an audio note

                                // Send typing indicator
                                bot.send_chat_action(message.chat.id, Typing).await?;

                                // Get the file_id of the voice message
                                let file_id = &voice.file.id;

                                // Download the voice message
                                let file = bot.get_file(file_id).await?;

                                // Convert to bytes
                                let file_path = file.path.clone();
                                let mut buffer = Vec::new();
                                info!("Downloading file to buffer");
                                bot.download_file(&file_path, &mut buffer).await?;

                                let voice_type: Mime = voice
                                    .mime_type
                                    .clone()
                                    .unwrap_or("audio/ogg".parse().unwrap());

                                // Transcribe the voice message
                                let translation =
                                    transcribe_audio(buffer, voice_type, TranscribeType::Translate)
                                        .await;

                                match translation {
                                    Ok(translation) => {
                                        // Send the translation to the user
                                        bot.send_message(message.chat.id, translation)
                                            .reply_to_message_id(message.id)
                                            .await?;
                                    }
                                    Err(e) => {
                                        error!("Failed to translate audio: {}", e);
                                        bot.send_message(
                                        message.chat.id,
                                        format!(
                                            "Failed to translate audio. Please try again later. ({e})"
                                        ),
                                    )
                                    .reply_to_message_id(message.id)
                                    .await?;
                                        return Ok(Response::builder()
                                            .status(200)
                                            .body(Body::Text(format!(
                                                "Failed to translate audio: {e}"
                                            )))
                                            .unwrap());
                                    }
                                }
                            } else {
                                bot.send_message(
                                    message.chat.id,
                                    "Please reply to a voice message with the /english command.",
                                )
                                .reply_to_message_id(message.id)
                                .await?;
                            }
                        } else {
                            bot.send_message(
                                message.chat.id,
                                "Please reply to a voice message with the /english command.",
                            )
                            .reply_to_message_id(message.id)
                            .await?;
                        }
                    } else if text.starts_with("/help")
                        || text.starts_with("/help@duck_transcriber_bot")
                    {
                        // Send help message
                        bot.send_message(
                            message.chat.id,
                            "Welcome to Duck Transcriber! Here are the available commands:
/tts <text> - Generate a voice message from text (reply to a message to use that text)
/english - Translate a voice message to English (reply to a voice message to use this command)",
                        )
                        .reply_to_message_id(message.id)
                        .await?;
                    };

                    info!("Unrecognized command");
                    Ok(Response::builder()
                        .status(200)
                        .body(Body::Text("Received unrecognized command".into()))
                        .unwrap())
                }
                MessageInfo { is_voice: true, .. } => {
                    // Get the voice duration
                    let duration = if let Some(voice) = message.voice() {
                        voice.duration
                    } else {
                        info!("Message is not a voice message");
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

                    let voice_id = message.voice().unwrap().file.id.clone();

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
                    let mut text = match openai::transcribe_audio(
                        buffer,
                        voice_type,
                        TranscribeType::Transcribe,
                    )
                    .await
                    {
                        Ok(text) => text,
                        Err(e) => {
                            info!("Failed to transcribe audio: {}", e);
                            bot.send_message(
                                message.chat.id,
                                format!(
                                    "Failed to transcribe audio. Please try again later. ({e})"
                                ),
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
                MessageInfo {
                    is_video_note: true,
                    ..
                } => {
                    // TODO!
                    // For now just return OK
                    Ok(Response::builder()
                        .status(200)
                        .body(Body::Text("OK".into()))
                        .unwrap())
                }
                _ => {
                    info!("Received unsupported message");
                    Ok(Response::builder()
                        .status(200)
                        .body(Body::Text("Received unsupported message".into()))
                        .unwrap())
                }
            }
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
