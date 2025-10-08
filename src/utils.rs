use lambda_http::{Body, Request};
use log::{info, warn};
use serde_json::Error;
use std::env;
use teloxide::{
    Bot,
    payloads::{SendDocumentSetters, SendMessageSetters},
    prelude::Requester,
    types::{ChatAction, ChatId, InputFile, Message, ParseMode, ReplyParameters, Update},
};
use tokio::time::{Duration, sleep};

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
    long_content_label: Option<&str>,
) {
    // Send the content to the user
    let content = content.unwrap_or("<no text>").trim().to_string();

    // Check the content length
    if content.len() > 4096 {
        info!("Content is too long, sending as a file instead of multiple messages");

        // Decide label and filename based on provided label
        let label = long_content_label.unwrap_or("content");
        let filename = match label {
            "transcript" => "transcript.txt",
            "translation" => "translation.txt",
            "summarization" | "summary" => "summary.txt",
            _ => "content.txt",
        };

        let caption = format!("Your {} is too long. Here is the file:", label);

        let file = InputFile::memory(content.into_bytes()).file_name(filename.to_string());

        let bot_msg = bot
            .send_document(message.chat.id, file)
            .caption(caption)
            .reply_parameters(ReplyParameters::new(message.id))
            .disable_notification(true)
            .await;

        if let Err(err) = bot_msg {
            warn!("Failed to send document: {err}");
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
            warn!("Failed to send message: {err}");
        }
    }
}

/// Starts a background task that sends Telegram "typing" action every 5 seconds
/// to indicate the bot is processing. Returns a guard that stops the heartbeat
/// when dropped.
pub struct TypingIndicatorGuard {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TypingIndicatorGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn start_typing_indicator(bot: Bot, chat_id: ChatId) -> TypingIndicatorGuard {
    let task = tokio::spawn(async move {
        // Send immediately, then every 5 seconds
        if let Err(err) = bot.send_chat_action(chat_id, ChatAction::Typing).await {
            warn!("Failed to send typing indicator: {err}");
        }

        loop {
            sleep(Duration::from_secs(5)).await;
            if let Err(err) = bot.send_chat_action(chat_id, ChatAction::Typing).await {
                warn!("Failed to send typing indicator: {err}");
            }
        }
    });

    TypingIndicatorGuard { task }
}

pub fn get_api_keys() -> Vec<String> {
    match env::var("GROQ_API_KEY") {
        Ok(keys_str) => {
            let keys: Vec<String> = keys_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            if keys.is_empty() {
                warn!("GROQ_API_KEY is empty");
                vec![]
            } else {
                info!("Loaded {} API key(s)", keys.len());
                keys
            }
        }
        Err(_) => {
            warn!("GROQ_API_KEY environment variable not set");
            vec![]
        }
    }
}
