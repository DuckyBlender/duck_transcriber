use lambda_http::{Body, Response};
use lambda_runtime::Error;
use teloxide::types::ParseMode;
use teloxide::{payloads::SendMessageSetters, requests::Requester, Bot};

pub async fn handle_help_command(
    bot: Bot,
    message: teloxide::types::Message,
) -> Result<Response<Body>, Error> {
    // Send help message
    bot.send_message(
        message.chat.id,
        "Welcome to Duck Transcriber! By default, the bot will transcribe every voice message and video note up to 5 minutes. Here are the available commands:

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
