// use crate::utils::dynamodb::stats;
use lambda_http::{Body, Response};
use lambda_runtime::Error;
use teloxide::types::ParseMode;
use teloxide::{payloads::SendMessageSetters, requests::Requester, Bot};
use tracing::error;

pub async fn handle_stats_command(
    bot: Bot,
    message: teloxide::types::Message,
    dynamodb_client: &aws_sdk_dynamodb::Client,
) -> Result<Response<Body>, Error> {
    // Get the user_id and username
    let user_id = message.from().unwrap().id;
    let username = message
        .from()
        .unwrap()
        .username
        .clone()
        .unwrap_or("".to_string());

    let stats = stats(dynamodb_client, user_id, username).await;

    match stats {
        Ok(stats) => {
            bot.send_message(message.chat.id, stats)
                .reply_to_message_id(message.id)
                .disable_web_page_preview(true)
                .disable_notification(true)
                .allow_sending_without_reply(true)
                .parse_mode(ParseMode::Html)
                .await?;
        }
        Err(e) => {
            error!("Failed to get stats: {}", e);
            bot.send_message(
                message.chat.id,
                format!("Failed to get stats. Please try again later. ({})", e),
            )
            .reply_to_message_id(message.id)
            .disable_web_page_preview(true)
            .allow_sending_without_reply(true)
            .await?;
        }
    }

    Ok(Response::builder()
        .status(200)
        .body(Body::Text("OK".into()))
        .unwrap())
}
