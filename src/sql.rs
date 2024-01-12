// /sql command for the telegram bot for executing mysql commands to the RDS database

use anyhow::Result;
use sqlx::{MySql, Pool};
use teloxide::{requests::Requester, types::Message, Bot};

use crate::utils::is_owner;

pub async fn handle_sql_command(
    bot: &Bot,
    message: &Message,
    command: String,
    pool: &Pool<MySql>,
) -> Result<()> {
    // Check if the user is an admin
    if !is_owner(message).await? {
        // bot.send_message(message.chat.id, "You are not an admin")
        // .await?;
        return Ok(());
    }

    // Get the command
    let command = command.replace("/sql ", "");

    // Execute the command
    let result = sqlx::query(&command)
        .execute(pool)
        .await
        .expect("Failed to execute SQL command");

    // Send the result to the user
    bot.send_message(message.chat.id, format!("{:?}", result))
        .await?;

    Ok(())
}
