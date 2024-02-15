use crate::utils::{
    dynamodb::{smart_add_item, Item, TABLE_NAME},
    openai::{transcribe_audio, OpenAIError, TranscribeType},
    other::TranscriptionData,
};
use lambda_http::{Body, Response};
use lambda_runtime::Error;
use mime::Mime;
use teloxide::types::ChatAction;
use teloxide::{net::Download, payloads::SendMessageSetters, requests::Requester, Bot};
use tracing::{error, info, warn};

pub async fn handle_voice_message(
    bot: Bot,
    message: teloxide::types::Message,
    dynamodb_client: &aws_sdk_dynamodb::Client,
) -> Result<Response<Body>, Error> {
    // Now that we know that the voice message is shorter then x minutes, download it and send it to openai
    // Send "typing" action to user
    bot.send_chat_action(message.chat.id, ChatAction::Typing)
        .await?;

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
        match transcribe_audio(buffer, voice_type, TranscribeType::Transcribe, duration).await {
            Ok(text) => text,
            Err(e) => {
                warn!("Failed to transcribe audio: {}", e);
                if e == OpenAIError::QuotaExceeded {
                    // Don't send any message
                    warn!("Failed to transcribe audio: Quota exceeded");
                    return Ok(Response::builder()
                        .status(200)
                        .body(Body::Text(
                            "Failed to transcribe audio: Quota exceeded".into(),
                        ))
                        .unwrap());
                }
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

    // Insert data into dynamodb
    let transcription_data = TranscriptionData {
        // convert user_id to u64
        user_id: message
            .from()
            .unwrap()
            .id
            .to_string()
            .parse::<u64>()
            .unwrap(),
        timestamp: message.date.to_string(),
        seconds_transcribed: duration as u64,
    };

    info!("Inserting voice note transcription data into dynamodb");

    let item = Item {
        table: TABLE_NAME.to_string(),
        user_id: transcription_data.user_id.to_string(),
        transcribed_seconds: transcription_data.seconds_transcribed,
    };

    match smart_add_item(dynamodb_client, item.clone()).await {
        Ok(_) => {
            info!("Successfully inserted data into dynamodb");
        }
        Err(e) => {
            error!("Failed to insert data into dynamodb: {}", e);
            error!("debug: {:?}", e);
            error!("item: {:?}", item);
        }
    }

    Ok(Response::builder()
        .status(200)
        .body(Body::Text("OK".into()))
        .unwrap())
}
