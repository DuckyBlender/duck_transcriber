use crate::types::TaskType;
use lambda_http::{Body, Request};
use log::{info, warn};
use serde_json::Error;
use std::env;
use teloxide::{
    Bot, payloads::{SendDocumentSetters, SendMessageSetters}, prelude::Requester, sugar::request::RequestReplyExt, types::{ChatAction, ChatId, InputFile, Message, ParseMode, Update}
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
    task_type: Option<TaskType>,
) {
    // Send the content to the user
    let content = content.unwrap_or("<no text>").trim().to_string();

    // Check the content length
    if content.len() > 4096 {
        info!("Content is too long, sending as a file instead of multiple messages");

        // Decide label and filename based on provided task type
        let (label, filename) = match task_type {
            Some(TaskType::Transcribe) => ("transcript", "transcript.txt"),
            Some(TaskType::Translate) => ("translation", "translation.txt"),
            Some(TaskType::Summarize) => ("summary", "summary.txt"),
            None => ("content", "content.txt"),
        };

        let caption = format!("Your {} is too long. Here is the file:", label);

        let file = InputFile::memory(content.into_bytes()).file_name(filename.to_string());

        let bot_msg = bot
            .send_document(message.chat.id, file)
            .caption(caption)
            .reply_to(message.id)
            .disable_notification(true)
            .await;

        if let Err(err) = bot_msg {
            warn!("Failed to send document: {err}");
        }
    } else {
        let mut bot_msg = bot
            .send_message(message.chat.id, &content)
            .reply_to(message.id)
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

/// Convert model identifiers into a human-friendly display name.
pub fn pretty_model_name(input: &str) -> String {
    // Take the last path segment if a repository/path prefix is present
    let segment = input.split('/').next_back().unwrap_or(input);

    // Normalize separators to dashes, then split on dash
    let normalized = segment.replace('_', "-");
    let parts = normalized.split('-');

    let mut out_parts: Vec<String> = Vec::new();
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }

        let lower = p.to_lowercase();

        // Drop tokens that are purely numeric (e.g., "0905")
        if lower.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        // Drop known boilerplate tokens
        if lower == "instruct" {
            continue;
        }

        // Capitalize first character, preserve the rest as-is
        let mut chars = p.chars();
        if let Some(first) = chars.next() {
            let mut s = String::new();
            s.extend(first.to_uppercase());
            s.push_str(chars.as_str());
            out_parts.push(s);
        }
    }

    if out_parts.is_empty() {
        // Fallback: just capitalize the whole input
        let mut chars = input.chars();
        if let Some(first) = chars.next() {
            let mut s = String::new();
            s.extend(first.to_uppercase());
            s.push_str(chars.as_str());
            return s;
        }
        return input.to_string();
    }

    out_parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::pretty_model_name;

    #[test]
    fn test_whisper() {
        assert_eq!(pretty_model_name("whisper-large-v3"), "Whisper Large V3");
    }

    #[test]
    fn test_moonshotai() {
        assert_eq!(
            pretty_model_name("moonshotai/kimi-k2-instruct-0905"),
            "Kimi K2"
        );
    }
}
