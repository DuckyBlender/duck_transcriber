use lambda_http::{Body, Request};
use log::{info, warn};
use serde_json::Error;
use teloxide::{
    Bot,
    payloads::SendMessageSetters,
    prelude::Requester,
    types::{Message, ParseMode, ReplyParameters, Update},
};

pub async fn parse_webhook(input: Request) -> Result<Update, Error> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        not => panic!("expected Body::Text(...) got {not:?}"),
    };
    let body_json: Update = serde_json::from_str(body_str)?;
    Ok(body_json)
}

pub async fn safe_send(
    bot: &Bot,
    message: &Message,
    content: Option<&str>,
    parse_mode: Option<ParseMode>,
) {
    // Send the content to the user
    let content = content.unwrap_or("<no text>").trim().to_string();

    // Check the content length
    if content.len() > 4096 {
        info!("Transcription is too long, splitting into multiple messages");
        let parts = split_string(&content, 4096);
        for part in parts {
            let bot_msg: Result<Message, teloxide::RequestError> = bot
                .send_message(message.chat.id, &part)
                .reply_parameters(ReplyParameters::new(message.id))
                // no parse mode here since we are splitting it and it would break the markdown
                .disable_notification(true)
                .await;
            // Send the message and handle error
            if let Err(err) = bot_msg {
                warn!("Failed to send message: {}", err);
            }
        }
    } else {
        let mut bot_msg = bot
            .send_message(message.chat.id, &content)
            .reply_parameters(ReplyParameters::new(message.id))
            .disable_notification(true);

        if let Some(parse_mode) = parse_mode {
            bot_msg = bot_msg.parse_mode(parse_mode);
        }

        let bot_msg = bot_msg.await;

        // Send the message and handle error
        if let Err(err) = bot_msg {
            warn!("Failed to send message: {}", err);
        }
    }
}

fn split_string(input: &str, max_length: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_chunk = String::new();
    let mut current_length = 0;

    for word in input.split_whitespace() {
        if current_length + word.len() + 1 > max_length && !current_chunk.is_empty() {
            result.push(current_chunk);
            current_chunk = String::new();
            current_length = 0;
        }

        if current_length > 0 {
            current_chunk.push(' ');
            current_length += 1;
        }

        current_chunk.push_str(word);
        current_length += word.len();
    }

    if !current_chunk.is_empty() {
        result.push(current_chunk);
    }

    result
}
