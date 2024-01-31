use crate::openai::{self, TranscribeType, Voice};
use crate::openai::{transcribe_audio, tts};
use crate::utils;
use lambda_http::{Body, Error, Request, Response};
use mime::Mime;
use std::env;
use std::fmt::{Debug, Display, Formatter};
use std::sync::Arc;
use teloxide::payloads::SendVoiceSetters;
use teloxide::types::ChatAction::{RecordVoice, Typing};
use teloxide::types::{InputFile, ParseMode};
use teloxide::{
    net::Download, payloads::SendMessageSetters, requests::Requester, types::UpdateKind, Bot,
};
use tokio::sync::Mutex;
use tracing::{error, info};

#[derive(Debug)]
pub struct MessageInfo {
    pub is_text: bool,
    pub is_voice: bool,
    pub is_video_note: bool,
}

impl Display for MessageInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Message: (is_text: {}, is_voice: {}, is_video_note: {})",
            self.is_text, self.is_voice, self.is_video_note
        )
    }
}

pub async fn handle_telegram_request(
    req: Request,
    bot: Arc<Mutex<Bot>>,
) -> Result<Response<Body>, Error> {
    // set the default
    let update = utils::convert_input_to_json(req).await.unwrap();

    // unwrap the bot
    let bot = bot.lock().await;

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
                    handle_text_message(bot.clone(), message).await
                }
                MessageInfo { is_voice: true, .. } => {
                    handle_voice_message(bot.clone(), message).await
                }
                MessageInfo {
                    is_video_note: true,
                    ..
                } => handle_video_note_message(bot.clone(), message).await,
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

async fn handle_text_message(
    bot: Bot,
    message: teloxide::types::Message,
) -> Result<Response<Body>, Error> {
    info!("Received text message");
    // Get the text from the message
    let text = message.text().unwrap();

    if text.starts_with("/tts") || text.starts_with("/tts@duck_transcriber_bot") {
        handle_tts_command(bot, message.clone(), text).await
    } else if text.starts_with("/english") || text.starts_with("/english@duck_transcriber_bot") {
        handle_english_command(bot, message).await
    } else if text.starts_with("/help") || text.starts_with("/help@duck_transcriber_bot") {
        handle_help_command(bot, message).await
    } else {
        info!("Unrecognized command");
        Ok(Response::builder()
            .status(200)
            .body(Body::Text("Received unrecognized command".into()))
            .unwrap())
    }
}

async fn handle_tts_command(
    bot: Bot,
    message: teloxide::types::Message,
    text: &str,
) -> Result<Response<Body>, Error> {
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
        .disable_web_page_preview(true)
        .allow_sending_without_reply(true)
        .await?;
        return Ok(Response::builder()
            .status(200)
            .body(Body::Text("No text provided".into()))
            .unwrap());
    }

    // Send "recording voice message" action to user
    bot.send_chat_action(message.chat.id, RecordVoice).await?;

    // random voice using rand
    let random_voice = match rand::random::<u8>() % 6 {
        0 => Voice::Alloy,
        1 => Voice::Echo,
        2 => Voice::Fable,
        3 => Voice::Onyx,
        4 => Voice::Nova,
        5 => Voice::Shimmer,
        _ => Voice::Alloy,
    };

    let voice = tts(tts_text.to_string(), random_voice).await;

    match voice {
        Ok(voice) => {
            // Send the voice message to the user
            bot.send_voice(message.chat.id, InputFile::memory(voice))
                .caption(format!("Voice: {}", tts_text))
                .reply_to_message_id(message.id)
                .await?;
        }
        Err(e) => {
            error!("Failed to generate voice: {}", e);
            bot.send_message(
                message.chat.id,
                format!("Failed to generate voice. Please try again later. ({e})"),
            )
            .reply_to_message_id(message.id)
            .disable_web_page_preview(true)
            .allow_sending_without_reply(true)
            .await?;

            return Ok(Response::builder()
                .status(200)
                .body(Body::Text(format!("Failed to generate voice: {e}")))
                .unwrap());
        }
    }

    Ok(Response::builder()
        .status(200)
        .body(Body::Text("OK".into()))
        .unwrap())
}

async fn handle_english_command(
    bot: Bot,
    message: teloxide::types::Message,
) -> Result<Response<Body>, Error> {
    // WE NEED AN AUDIO INPUT HERE
    // USE THE AUDIO FROM THE REPLY
    if let Some(reply) = message.reply_to_message() {
        if let Some(voice) = reply.voice() {
            // Send typing indicator
            bot.send_chat_action(message.chat.id, Typing).await?;

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

            let voice_type: Mime = voice
                .mime_type
                .clone()
                .unwrap_or("audio/ogg".parse().unwrap());

            // Transcribe the voice message
            let translation =
                transcribe_audio(buffer, voice_type, TranscribeType::Translate, duration).await;

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
            bot.send_chat_action(message.chat.id, Typing).await?;

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

            let voice_type: Mime = "audio/mp4".parse().unwrap();

            // Transcribe the voice message
            let translation =
                transcribe_audio(buffer, voice_type, TranscribeType::Translate, duration).await;

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

async fn handle_help_command(
    bot: Bot,
    message: teloxide::types::Message,
) -> Result<Response<Body>, Error> {
    // Send help message
    bot.send_message(
        message.chat.id,
        "Welcome to Duck Transcriber! By default, the bot will transcribe every voice message and video note up to 5 minutes. Here are the available commands:

<code>/tts</code> - Generate a voice message from argument (reply to a message to use that text)
<code>/english</code> - Translate a voice message to English (reply to a voice message to use this command)",
    )
    .reply_to_message_id(message.id)
    .disable_web_page_preview(true)
    .allow_sending_without_reply(true)
    .parse_mode(ParseMode::Html)
    .await?;

    Ok(Response::builder()
        .status(200)
        .body(Body::Text("OK".into()))
        .unwrap())
}

async fn handle_voice_message(
    bot: Bot,
    message: teloxide::types::Message,
) -> Result<Response<Body>, Error> {
    // Now that we know that the voice message is shorter then x minutes, download it and send it to openai
    // Send "typing" action to user
    bot.send_chat_action(message.chat.id, Typing).await?;

    let voice_id = message.voice().unwrap().file.id.clone();

    // Length of the voice message
    let duration = message.voice().unwrap().duration;

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
    let mut text =
        match openai::transcribe_audio(buffer, voice_type, TranscribeType::Transcribe, duration)
            .await
        {
            Ok(text) => text,
            Err(e) => {
                info!("Failed to transcribe audio: {}", e);
                bot.send_message(
                    message.chat.id,
                    format!("Failed to transcribe audio. Please try again later. ({e})"),
                )
                .reply_to_message_id(message.id)
                .disable_web_page_preview(true)
                .allow_sending_without_reply(true)
                .await?;
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::Text(format!("Failed to transcribe audio: {e}")))
                    .unwrap());
            }
        };

    if text.is_empty() || text == "you" {
        // for some reason, if nothing is said it returns "you"
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

async fn handle_video_note_message(
    bot: Bot,
    message: teloxide::types::Message,
) -> Result<Response<Body>, Error> {
    // Check if the video note is present
    let video_note = if let Some(video_note) = message.video_note() {
        video_note
    } else {
        info!("Message is not a video note");
        return Ok(Response::builder()
            .status(200)
            .body(Body::Text("Message is not a video note".into()))
            .unwrap());
    };

    // Send "typing" action to user
    bot.send_chat_action(message.chat.id, Typing).await?;

    let video_note_id = video_note.file.id.clone();

    // Length of the voice message
    let duration = video_note.duration;

    // Get the video note mime type
    let default_mime: Mime = "audio/mp4".parse().unwrap();

    let file = bot.get_file(video_note_id).await?;
    let file_path = file.path.clone();
    let mut buffer = Vec::new();
    info!("Downloading file to buffer");
    bot.download_file(&file_path, &mut buffer).await?;

    // Send file to OpenAI Whisper for transcription
    let mut text =
        match openai::transcribe_audio(buffer, default_mime, TranscribeType::Transcribe, duration)
            .await
        {
            Ok(text) => text,
            Err(e) => {
                info!("Failed to transcribe audio: {}", e);
                bot.send_message(
                    message.chat.id,
                    format!("Failed to transcribe audio. Please try again later. ({e})"),
                )
                .reply_to_message_id(message.id)
                .allow_sending_without_reply(true)
                .disable_web_page_preview(true)
                .await?;
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::Text(format!("Failed to transcribe audio: {e}")))
                    .unwrap());
            }
        };

    if text.is_empty() || text == "you" {
        // for some reason, if nothing is said it returns "you"
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

async fn send_dev_message(bot: Bot, message: String) {
    let dev_chat_id = env::var("TELEGRAM_OWNER_ID").unwrap();
    bot.send_message(dev_chat_id, message)
        .disable_web_page_preview(true)
        .await
        .unwrap();
}
