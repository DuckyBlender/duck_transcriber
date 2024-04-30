use lambda_runtime::Error;
use lambda_runtime::LambdaEvent;
use std::fmt::Display;
use std::fmt::Formatter;
use teloxide::{
    requests::Requester,
    types::{BotCommand, Update},
    Bot,
};
use tracing::error;

pub async fn convert_input_to_json(input: LambdaEvent<serde_json::Value>) -> Result<Update, Error> {
    // The serde_json::Value is an object. To get the telegram information we need the body, which is a string (JSON)
    if input.payload.is_null() {
        error!("Payload is null");
        return Err(Error::from("Payload is null"));
    }

    // Check if the serde_json::Value is a JSON object
    if !input.payload.is_object() {
        error!("Payload is not a JSON object");
        return Err(Error::from("Payload is not a JSON object".to_string()));
    }

    // Get the body of the JSON object
    let body = input.payload.get("body").ok_or_else(|| {
        error!("Failed to get body from input");
        Error::from("Failed to get body from input")
    })?;

    // Convert to str
    let body = body.as_str().ok_or_else(|| {
        error!("Failed to convert body to str");
        Error::from("Failed to convert body to str")
    })?;

    let update: Update = serde_json::from_str(body).map_err(|e| {
        error!("Failed to convert body to json: {}", e);
        Error::from("Failed to convert body to json".to_string())
    })?;

    Ok(update)
}

#[rustfmt::skip]
pub async fn set_commands(bot: &Bot) -> Result<teloxide::types::True, teloxide::RequestError> {
    let commands = vec![
        BotCommand::new("tts", "Text to speech a message using OpenAI's TTS"),
        BotCommand::new("english", "Translate a voice message to english using OpenAI Translations"),
        BotCommand::new("help", "Show the help message"),
        BotCommand::new("stats", "Show the stats of the user"),
    ];

    bot.set_my_commands(commands).await
}

#[derive(Debug)]
pub struct MessageInfo {
    pub is_text: bool,
    pub is_voice: bool,
    pub is_video_note: bool,
}

pub struct TranscriptionData {
    pub user_id: u64,
    pub timestamp: String,
    pub seconds_transcribed: u64,
}

impl Display for MessageInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Message: (is_text: {}, is_voice: {}, is_video_note: {})",
            self.is_text, self.is_voice, self.is_video_note
        )
    }
}
