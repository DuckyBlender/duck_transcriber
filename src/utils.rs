use crate::types::TaskType;
use log::{error, info, warn};
use serde::de::DeserializeOwned;
use teloxide::{
    Bot, RequestError,
    payloads::{SendDocumentSetters, SendMessageSetters, SetMessageReactionSetters},
    prelude::Requester,
    sugar::request::RequestReplyExt,
    types::{ChatAction, ChatId, InputFile, Message, ParseMode, ReactionType},
    utils::html,
};
use tokio::time::{Duration, sleep};

const MAX_MESSAGE_LEN: usize = 4096;
// How much of an unexpected response body to include in the log.
const MAX_LOGGED_BODY_CHARS: usize = 2000;

/// Sends the result of `task`: summaries in italics; transcripts and
/// translations in an expandable quote in groups, so long ones stay short until
/// someone taps them, and as plain text in DMs.
pub async fn send_result(bot: &Bot, message: &Message, text: &str, task: TaskType) {
    let text = text.trim();
    // The length check runs on the plain text so an oversized result is sent as
    // a file without the HTML markup.
    if text.len() > MAX_MESSAGE_LEN {
        let label = match task {
            TaskType::Transcribe | TaskType::Phonon => "transcript",
            TaskType::Translate => "translation",
            TaskType::Summarize | TaskType::Caveman => "summary",
        };
        info!("The {label} is too long, sending it as a file");
        let file = InputFile::memory(text.as_bytes().to_vec()).file_name(format!("{label}.txt"));
        let sent = bot
            .send_document(message.chat.id, file)
            .caption(format!("Your {label} is too long. Here is the file:"))
            .reply_to(message.id)
            .disable_notification(true)
            .await;
        if let Err(err) = sent {
            warn!("Failed to send document: {err}");
        }
        return;
    }

    match task {
        TaskType::Summarize | TaskType::Caveman => {
            let italic = format!("<i>{}</i>", html::escape(text));
            reply(bot, message, &italic, Some(ParseMode::Html)).await;
        }
        _ if message.chat.is_private() => reply(bot, message, text, None).await,
        _ => reply(bot, message, &expandable_quote(text), Some(ParseMode::Html)).await,
    }
}

pub async fn reply(bot: &Bot, message: &Message, text: &str, parse_mode: Option<ParseMode>) {
    let mut request = bot
        .send_message(message.chat.id, text)
        .reply_to(message.id)
        .disable_notification(true);

    if let Some(parse_mode) = parse_mode {
        request = request.parse_mode(parse_mode);
    }

    if let Err(err) = request.await {
        warn!("Failed to send message: {err}");
    }
}

pub async fn set_reaction(bot: &Bot, message: &Message, emoji: &str) {
    let reaction = ReactionType::Emoji {
        emoji: emoji.to_string(),
    };
    if let Err(err) = bot
        .set_message_reaction(message.chat.id, message.id)
        .reaction([reaction])
        .await
    {
        error!("Failed to set reaction: {err:?}");
    }
}

/// Stops the "typing" heartbeat started by `start_typing_indicator` when dropped.
pub struct TypingIndicatorGuard {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TypingIndicatorGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Sends the "typing" action now and then every 5 seconds until the returned
/// guard is dropped. Returns `None` when Telegram rejects the first one, which
/// means the bot can't post in the chat (muted, read-only group, kicked), so
/// there is no point transcribing anything for it.
pub async fn start_typing_indicator(bot: Bot, chat_id: ChatId) -> Option<TypingIndicatorGuard> {
    match bot.send_chat_action(chat_id, ChatAction::Typing).await {
        Err(RequestError::Api(err)) => {
            info!("Skipping chat {chat_id}, the bot can't post there: {err}");
            return None;
        }
        // Network errors and flood waits say nothing about permissions.
        Err(err) => warn!("Failed to send typing indicator: {err}"),
        Ok(_) => {}
    }

    let task = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(5)).await;
            if let Err(err) = bot.send_chat_action(chat_id, ChatAction::Typing).await {
                warn!("Failed to send typing indicator: {err}");
            }
        }
    });

    Some(TypingIndicatorGuard { task })
}

fn expandable_quote(text: &str) -> String {
    format!("<blockquote expandable>{}</blockquote>", html::escape(text))
}

/// Sends a request to one of the local servers (whisper.cpp, Phonon or
/// llama.cpp) and parses its JSON answer. Failures are logged here with the
/// start of the body, so callers only decide how to react.
pub async fn post_json<T: DeserializeOwned>(
    server: &str,
    request: reqwest::RequestBuilder,
) -> Result<T, String> {
    let res = request.send().await.map_err(|err| {
        error!("Failed to send request to {server}: {err}");
        format!("Failed to send request to {server}: {err}")
    })?;

    let status = res.status();
    let body = res.text().await.map_err(|err| {
        error!("Failed to read {server} response body ({status}): {err}");
        format!("Failed to read {server} response body: {err}")
    })?;

    if !status.is_success() {
        error!("{server} returned {status}: {}", truncate_for_log(&body));
        return Err(format!("{server} error: {status}"));
    }

    serde_json::from_str(&body).map_err(|err| {
        error!(
            "Failed to parse {server} response: {err}; body: {}",
            truncate_for_log(&body)
        );
        format!("Failed to parse {server} response")
    })
}

pub fn truncate_for_log(body: &str) -> String {
    let mut chars = body.chars();
    let mut truncated: String = chars.by_ref().take(MAX_LOGGED_BODY_CHARS).collect();
    if chars.next().is_some() {
        truncated.push_str("… (truncated)");
    }
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_escapes_transcripts() {
        assert_eq!(
            expandable_quote("<no text> & more"),
            "<blockquote expandable>&lt;no text&gt; &amp; more</blockquote>"
        );
    }

    #[test]
    fn truncates_long_bodies_on_char_boundaries() {
        let body = "ż".repeat(MAX_LOGGED_BODY_CHARS + 5);
        let logged = truncate_for_log(&body);
        assert!(logged.ends_with("… (truncated)"));
        assert_eq!(
            logged.chars().filter(|c| *c == 'ż').count(),
            MAX_LOGGED_BODY_CHARS
        );
        assert_eq!(truncate_for_log("short"), "short");
    }
}
