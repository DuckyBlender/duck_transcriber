use super::other::{convert_input_to_json, MessageInfo};
use crate::{
    listeners::{
        text::handle_text_message, video::handle_video_note_message, voice::handle_voice_message,
    },
    Response,
};
use lambda_runtime::{Error, LambdaEvent};
use serde_json::Value as JsonValue;
use teloxide::{types::UpdateKind, Bot};
use tracing::error;
use tracing::info;

pub async fn handle_telegram_request(
    req: LambdaEvent<JsonValue>,
    bot: &Bot,
    dynamodb_client: &aws_sdk_dynamodb::Client,
) -> Result<Response, Error> {
    // set the default
    let update = convert_input_to_json(req).await;
    if let Err(e) = update {
        error!("Failed to convert input to json: {}", e);
        return Ok(Response {
            body: "Failed to convert input to json".to_string(),
        });
    };

    // safe to unwrap
    let update = update.unwrap();

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
                    handle_text_message(bot.clone(), message, dynamodb_client).await
                }
                MessageInfo { is_voice: true, .. } => {
                    handle_voice_message(bot.clone(), message, dynamodb_client).await
                }
                MessageInfo {
                    is_video_note: true,
                    ..
                } => handle_video_note_message(bot.clone(), message, dynamodb_client).await,
                _ => {
                    info!("Received unsupported message");
                    Ok(Response {
                        body: "Received unsupported message".to_string(),
                    })
                }
            }
        }
        // If the update is not a message
        _ => {
            info!("Update is not a message");
            Ok(Response {
                body: "Update is not a message".to_string(),
            })
        }
    }
}
