// use crate::utils::dynamodb::stats;
use lambda_runtime::Error;
use teloxide::{payloads::SendMessageSetters, requests::Requester, types::ParseMode, Bot};
use tracing::info;

use crate::{
    utils::dynamodb::{query_item, Item, TABLE_NAME},
    Response,
};

pub async fn handle_stats_command(
    bot: Bot,
    message: teloxide::types::Message,
    dynamodb_client: &aws_sdk_dynamodb::Client,
) -> Result<Response, Error> {
    // Get the user_id and username
    let user_id = message.from().unwrap().id.0;
    let username = message
        .from()
        .unwrap()
        .username
        .clone()
        .unwrap_or("".to_string());

    let item = Item {
        table: TABLE_NAME.to_string(),
        user_id,
        transcribed_seconds: 0, // not used
    };
    let seconds = query_item(dynamodb_client, item).await;
    if seconds.is_none() {
        info!("User has no stats");
        bot.send_message(
            message.chat.id,
            "You have no stats. Start sending voice messages or video notes to get some!"
                .to_string(),
        )
        .reply_to_message_id(message.id)
        .disable_web_page_preview(true)
        .allow_sending_without_reply(true)
        .await?;
        return Ok(Response {
            body: "You have no stats. Start sending voice messages or video notes to get some!"
                .into(),
        });
    }
    let seconds = seconds.unwrap();

    // Convert to hours, minutes, and seconds
    let (hours, remainder) = (seconds / 3600, seconds % 3600);
    let (minutes, seconds) = (remainder / 60, remainder % 60);

    let time = if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    };

    // Send the stats to the user
    bot.send_message(
        message.chat.id,
        format!(
            "<b>Your stats:</b>\n- Username: <code>{}</code>\n- Transcribed: <code>{}</code>",
            username, time
        ),
    )
    .reply_to_message_id(message.id)
    .disable_web_page_preview(true)
    .allow_sending_without_reply(true)
    .parse_mode(ParseMode::Html)
    .await?;

    Ok(Response {
        body: format!("Stats for user {} sent", username),
    })
}