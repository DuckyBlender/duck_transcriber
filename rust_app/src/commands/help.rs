use lambda_runtime::Error;
use teloxide::types::ParseMode;
use teloxide::{payloads::SendMessageSetters, requests::Requester, Bot};

use crate::utils::openai::MINUTE_LIMIT;
use crate::Response;

pub async fn handle_help_command(
    bot: Bot,
    message: teloxide::types::Message,
) -> Result<Response, Error> {
    // Send help message
    bot.send_message(
        message.chat.id,
        format!("Welcome to Duck Transcriber! By default, the bot will transcribe every voice message and video note up to {} minutes. Here are the available commands:

<code>/tts</code> - Generate a voice message from argument (reply to a message to use this command)
<code>/english</code> - Translate a voice message to English (reply to a voice message to use this command)", MINUTE_LIMIT)
    )
    .reply_to_message_id(message.id)
    .disable_web_page_preview(true)
    .allow_sending_without_reply(true)
    .parse_mode(ParseMode::Html)
    .await?;

    Ok(Response {
        body: "Help message sent".into(),
    })
}
