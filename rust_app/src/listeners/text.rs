use crate::commands::help::handle_help_command;
use crate::commands::stats::handle_stats_command;
use crate::commands::tts::handle_tts_command;
use crate::{commands::english::handle_english_command, Response};
use lambda_runtime::Error;
use teloxide::{
    requests::{Request, Requester},
    Bot,
};
use tracing::info;

use teloxide::types::MessageEntityKind::BotCommand;

pub async fn handle_text_message(
    bot: Bot,
    message: teloxide::types::Message,
    dynamodb_client: &aws_sdk_dynamodb::Client, // for /stats command
) -> Result<Response, Error> {
    info!("Received text message");

    // Parse the command
    let command = message.parse_entities();
    if command.is_none() {
        info!("Ignoring photo, etc: {:?}", message);
        return Ok(Response {
            body: "Ignoring photo, etc".into(),
        });
    }
    let command = command.unwrap();
    // Check if there is a command
    if command.is_empty() {
        info!("Ignoring message: {:?}", message);
        return Ok(Response {
            body: "Ignoring message".into(),
        });
    }
    // Check if the first arguemnt is a command
    if *command.first().unwrap().kind() != BotCommand {
        info!("Ignoring message: {:?}", message);
        return Ok(Response {
            body: "Ignoring message".into(),
        });
    }
    // Check which command was sent
    // First we need to check if there is a @ in the command
    // If there is, we need to check if it's the bot's username and remove it
    let command = command.first().unwrap().text();
    let command = if command.contains('@') {
        let command = command.split('@').collect::<Vec<&str>>();
        if command[1] == bot.get_me().send().await.unwrap().user.username.unwrap() {
            command[0]
        } else {
            return Ok(Response {
                body: "Ignoring message: Command not for this bot".into(),
            });
        }
    } else {
        command
    };

    match command {
        "/stats" => handle_stats_command(bot, message, dynamodb_client).await,
        "/tts" => handle_tts_command(bot, message).await,
        "/english" => handle_english_command(bot, message).await,
        "/help" => handle_help_command(bot, message).await,
        _ => {
            info!("Ignoring message: {:?}", message);
            Ok(Response {
                body: "Ignoring message".into(),
            })
        }
    }
}
