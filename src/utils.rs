use crate::types::TaskType;
use log::{info, warn};
use teloxide::{
    Bot,
    payloads::{SendDocumentSetters, SendMessageSetters},
    prelude::Requester,
    sugar::request::RequestReplyExt,
    types::{ChatAction, ChatId, InputFile, Message, ParseMode},
    utils::html,
};
use tokio::time::{Duration, sleep};

const MAX_MESSAGE_LEN: usize = 4096;

/// Sends a transcript or translation. In groups it goes in an expandable quote
/// so long transcripts stay short until someone taps them; DMs get plain text.
pub async fn send_transcript(bot: &Bot, message: &Message, text: &str, task_type: TaskType) {
    let text = text.trim();
    // The length check runs on the plain text so an oversized transcript is
    // sent as a file without the HTML markup.
    if message.chat.is_private() || text.len() > MAX_MESSAGE_LEN {
        safe_send(bot, message, Some(text), None, Some(task_type)).await;
        return;
    }

    send_message(bot, message, &expandable_quote(text), Some(ParseMode::Html)).await;
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
    if content.len() > MAX_MESSAGE_LEN {
        info!("Content is too long, sending as a file instead of multiple messages");

        // Decide label and filename based on provided task type
        let (label, filename) = match task_type {
            Some(TaskType::Transcribe) | Some(TaskType::Parakeet) => {
                ("transcript", "transcript.txt")
            }
            Some(TaskType::Translate) => ("translation", "translation.txt"),
            Some(TaskType::Summarize) | Some(TaskType::Caveman) => ("summary", "summary.txt"),
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
        send_message(bot, message, &content, parse_mode).await;
    }
}

async fn send_message(bot: &Bot, message: &Message, content: &str, parse_mode: Option<ParseMode>) {
    let mut bot_msg = bot
        .send_message(message.chat.id, content)
        .reply_to(message.id)
        .disable_notification(true);

    if let Some(parse_mode) = parse_mode {
        bot_msg = bot_msg.parse_mode(parse_mode);
    }

    if let Err(err) = bot_msg.await {
        warn!("Failed to send message: {err}");
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

fn expandable_quote(text: &str) -> String {
    format!("<blockquote expandable>{}</blockquote>", html::escape(text))
}

#[cfg(test)]
mod tests {
    use super::expandable_quote;

    #[test]
    fn quotes_and_escapes_transcripts() {
        assert_eq!(
            expandable_quote("<no text> & more"),
            "<blockquote expandable>&lt;no text&gt; &amp; more</blockquote>"
        );
    }
}
