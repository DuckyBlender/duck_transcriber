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

pub async fn handle_video_note_message(
    bot: Bot,
    message: teloxide::types::Message,
    dynamodb_client: &aws_sdk_dynamodb::Client,
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
    bot.send_chat_action(message.chat.id, ChatAction::Typing)
        .await?;

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
        match transcribe_audio(buffer, default_mime, TranscribeType::Transcribe, duration).await {
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

    info!("Inserting video note transcription data into dynamodb");

    let item = Item {
        table: TABLE_NAME.to_string(),
        user_id: transcription_data.user_id.to_string(),
        transcribed_seconds: transcription_data.seconds_transcribed,
    };

    match smart_add_item(dynamodb_client, item).await {
        Ok(_) => info!("Successfully inserted video note transcription data into dynamodb"),
        Err(e) => {
            error!(
                "Failed to insert video note transcription data into dynamodb: {}",
                e
            );
            return Ok(Response::builder()
                .status(200)
                .body(Body::Text(
                    "Failed to insert video note transcription data into dynamodb".into(),
                ))
                .unwrap());
        }
    }

    Ok(Response::builder()
        .status(200)
        .body(Body::Text("OK".into()))
        .unwrap())
}
