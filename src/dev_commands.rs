use log::warn;
use serde::Deserialize;
use std::env;
use teloxide::types::{Message, UserId};
use teloxide::{Bot, prelude::Requester};

use crate::utils::safe_send;

#[derive(Deserialize, Debug)]
struct TelegramResponse<T> {
    ok: bool,
    result: T,
}

#[derive(Deserialize, Debug)]
struct WebhookInfo {
    pending_update_count: u32,
    last_error_message: Option<String>,
}

#[derive(Deserialize)]
struct OkOnly {
    ok: bool,
}

fn is_authorized_developer(message: &Message) -> bool {
    let author_id: Option<u64> = message.from.as_ref().map(|u| {
        let UserId(id) = u.id;
        id
    });

    let expected_id: Option<u64> = env::var("DEV_TELEGRAM_ID")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());

    matches!((author_id, expected_id), (Some(id), Some(exp_id)) if id == exp_id)
}

fn extract_command(message: &Message) -> Option<&str> {
    // Prefer text, fallback to caption
    message.text().or_else(|| message.caption())
}

fn is_command_match(cmd_text: &str, target: &str, bot_username: &str) -> bool {
    // Support both "/cmd" and "/cmd@botusername" forms as the first token
    let first_token = cmd_text.split_whitespace().next().unwrap_or("");
    first_token == target || first_token == format!("{}@{}", target, bot_username)
}

pub async fn try_handle_dev_command(bot: &Bot, message: &Message) -> bool {
    let Some(cmd_text) = extract_command(message) else {
        return false;
    };

    // Quick prefilter: commands we care about
    if !(cmd_text.starts_with("/check") || cmd_text.starts_with("/reset")) {
        return false;
    }

    // Authorization: stealth - do not respond if not authorized
    if !is_authorized_developer(message) {
        return false;
    }

    let me = match bot.get_me().await {
        Ok(me) => me,
        Err(err) => {
            warn!("Failed to get bot username: {err}");
            return false;
        }
    };
    let bot_username = me.username();

    // /check: print pending_update_count
    if is_command_match(cmd_text, "/check", bot_username) {
        if let Err(err) = handle_check(bot, message).await {
            warn!("/check failed: {err}");
        }
        return true;
    }

    // /reset: setWebhook with drop_pending_updates=true
    if is_command_match(cmd_text, "/reset", bot_username) {
        if let Err(err) = handle_reset(bot, message).await {
            warn!("/reset failed: {err}");
        }
        return true;
    }

    false
}

async fn handle_check(bot: &Bot, message: &Message) -> Result<(), String> {
    let token =
        env::var("TELEGRAM_BOT_TOKEN").map_err(|_| "TELEGRAM_BOT_TOKEN not set".to_string())?;
    let url = format!("https://api.telegram.org/bot{}/getWebhookInfo", token);

    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("telegram api http status: {}", resp.status()));
    }

    let json: TelegramResponse<WebhookInfo> = resp
        .json()
        .await
        .map_err(|e| format!("json parse error: {e}"))?;

    if !json.ok {
        return Err("telegram api responded with ok=false".to_string());
    }

    let mut response_lines = vec![format!(
        "Pending updates: {}",
        json.result.pending_update_count
    )];
    if let Some(err_msg) = json.result.last_error_message.as_deref() {
        response_lines.push(format!("Last error: {}", err_msg));
    }
    let msg = response_lines.join("\n");
    safe_send(bot, message, Some(&msg), None, None).await;
    Ok(())
}

async fn handle_reset(bot: &Bot, message: &Message) -> Result<(), String> {
    let token =
        env::var("TELEGRAM_BOT_TOKEN").map_err(|_| "TELEGRAM_BOT_TOKEN not set".to_string())?;
    let lambda_url = env::var("LAMBDA_URL").map_err(|_| "LAMBDA_URL not set".to_string())?;
    let url = format!("https://api.telegram.org/bot{}/setWebhook", token);

    let client = reqwest::Client::new();
    let params = [
        ("url", lambda_url.as_str()),
        ("allowed_updates", "[\"message\"]"),
        ("drop_pending_updates", "true"),
    ];

    let resp = client
        .post(url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("request error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("telegram api http status: {}", resp.status()));
    }

    let json: OkOnly = resp
        .json()
        .await
        .map_err(|e| format!("json parse error: {e}"))?;
    if !json.ok {
        return Err("telegram api responded with ok=false".to_string());
    }

    safe_send(
        bot,
        message,
        Some("Webhook reset: drop_pending_updates=true"),
        None,
        None,
    )
    .await;
    Ok(())
}
