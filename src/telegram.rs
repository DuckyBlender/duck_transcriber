use lambda_http::{Body, Error, Request, Response};

use std::env;
use teloxide::{
    net::Download,
    payloads::SendMessageSetters,
    requests::Requester,
    types::{ParseMode, UpdateKind},
    Bot,
};
use tracing::info;
use teloxide::types::ChatAction::Typing;

use crate::openai;
use crate::utils;

const MINUTE_LIMIT: u32 = 5;

pub async fn handle_telegram_request(req: Request) -> Result<Response<Body>, Error> {
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    let update = utils::convert_input_to_json(req).await?;
    info!("update: {:?}", update);

    // Match the update type
    match update.kind {
        // If the update is a message
        UpdateKind::Message(message) => {
            // Check if the message is a voice message
            if message.voice().is_none() {
                return Ok(Response::builder()
                    .status(200)
                    .body(Body::Text("Not a voice message".into()))
                    .unwrap());
            }

            let voice = message.voice().unwrap();

            // Check if voice message is longer than 1 minute
            if voice.duration > MINUTE_LIMIT {
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

            let voice_id = voice.file.id.clone();
            let file = bot.get_file(voice_id).await?;
            let file_path = file.path.clone();
            let mut buffer = Vec::new();
            info!("Downloading file: {:?}", file_path);
            bot.download_file(&file_path, &mut buffer).await?;
            info!("Downloaded file: {:?}", file_path);

            // Send file to OpenAI Whisper for transcription
            info!("Sending file to OpenAI Whisper for transcription");
            let text = openai::transcribe_audio(buffer).await?;
            info!("Received text from OpenAI Whisper: {:?}", text);

            let text = format!(
                "{text}\n<i>Powered by <a href=\"https://openai.com/research/whisper\">OpenAI Whisper</a></i>"
            );

            // Send text to user
            bot.send_message(message.chat.id, text)
                .reply_to_message_id(message.id)
                .parse_mode(ParseMode::Html)
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
