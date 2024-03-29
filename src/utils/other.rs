use lambda_http::{Body, Error, Request};
use std::fmt::Display;
use std::fmt::Formatter;
use teloxide::{
    requests::Requester,
    types::{BotCommand, Update},
    Bot,
};

pub async fn convert_input_to_json(input: Request) -> Result<Update, Error> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        not => panic!("expected Body::Text(...) got {not:?}"),
    };
    let body_json: Update = serde_json::from_str(body_str)?;
    Ok(body_json)
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
