use super::other::{convert_input_to_json, MessageInfo};
use crate::listeners::{
    text::handle_text_message, video::handle_video_note_message, voice::handle_voice_message,
};
use lambda_http::{Body, Request, Response};
use lambda_runtime::Error;
use teloxide::{types::UpdateKind, Bot};
use tracing::info;

pub async fn handle_telegram_request(
    req: Request,
    bot: &Bot,
    dynamodb_client: &aws_sdk_dynamodb::Client,
) -> Result<Response<Body>, Error> {
    // set the default
    let update = convert_input_to_json(req).await;
    if let Err(e) = update {
        info!("Failed to convert input to json: {}", e);
        return Ok(Response::builder()
            .status(200)
            .body(Body::Text(format!(
                "Failed to convert input to json: {}",
                e
            )))
            .unwrap());
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
