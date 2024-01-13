use sqlx::{MySql, Pool};
use teloxide::types::ChatId;

// TODO: REMOVE SQL INJECTION VULN LMAO

// The `transcriber` table in the database has the following structure:

// | Column Name | Data Type | Key | Default Value | Description |
// |-------------|-----------|-----|---------------|-------------|
// | chat_id     | BIGINT    | PRIMARY KEY | None | Unique identifier for the chat |
// | debug_info  | BOOLEAN   |     | false | Enables or disables debug information |
// | delete_voice| BOOLEAN   |     | false | Option to delete the original voice message after transcription |
// | gpt_enhance | BOOLEAN   |     | false | Enhances transcribed text with GPT |

use crate::utils::SqlCommands;
use crate::utils::State;

pub async fn handle_sql_command(
    pool: &Pool<MySql>,
    command: SqlCommands,
) -> Result<bool, sqlx::Error> {
    // Match the command
    let sql_command = match command {
        SqlCommands::DebugInfo(state, chat_id) => {
            generate_sql_command(&state, chat_id, "debug_info")
        }
        SqlCommands::RemoveOriginalVoice(state, chat_id) => {
            generate_sql_command(&state, chat_id, "delete_voice")
        }
        SqlCommands::GPTEnabled(state, chat_id) => {
            generate_sql_command(&state, chat_id, "gpt_enhance")
        }
    };

    // Execute the command
    let result = sqlx::query(&sql_command).execute(pool).await?;

    // Return true if rows were affected, false otherwise
    Ok(result.rows_affected() > 0)
}

fn generate_sql_command(state: &State, chat_id: ChatId, column_name: &str) -> String {
    // Convert chat ID to string
    let chat_id = chat_id.to_string();
    match state {
        State::Enable => format!(
            "INSERT INTO transcriber (chat_id, {}) VALUES ({}, true) ON DUPLICATE KEY UPDATE {} = true",
            column_name, chat_id, column_name
        ),
        State::Disable => format!(
            "INSERT INTO transcriber (chat_id, {}) VALUES ({}, false) ON DUPLICATE KEY UPDATE {} = false",
            column_name, chat_id, column_name
        ),
        State::Toggle => format!(
            "INSERT INTO transcriber (chat_id, {}) VALUES ({}, false) ON DUPLICATE KEY UPDATE {} = NOT {}",
            column_name, chat_id, column_name, column_name
        ),
    }
}
