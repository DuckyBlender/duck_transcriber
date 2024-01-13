use lambda_http::{Body, Request};
use teloxide::types::{ChatId, Update};

pub const HELP_MESSAGE: &str = r#"
Hello there! I'm Duck Transcriber, a bot which transcribes voice messages to text using the OpenAI Whisper API.
Here are the available commands:
`/help` - Show this help message
`/debug` - Enable or disable debug information
`/remove` - Enable or disable removing the original voice message after transcription
`/gpt_enhance` - Enable or disable enhancing the transcribed text with GPT (WIP)
"#;

pub enum SqlCommands {
    DebugInfo(State, ChatId),
    RemoveOriginalVoice(State, ChatId),
    #[allow(dead_code)] // This command is WIP
    GPTEnabled(State, ChatId),
}

pub enum State {
    Enable,
    Disable,
    Toggle,
}

// anyhow result
pub async fn convert_input_to_json(input: Request) -> Result<Update, String> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        _ => {
            return Err("Body is not text".to_string());
        }
    };
    let body_json: Update = serde_json::from_str(body_str)
        .map_err(|err| format!("Failed to parse body as JSON: {}", err))?;
    Ok(body_json)
}

// pub const TELEGRAM_OWNER_ID: u64 = 5337682436;
// pub async fn is_owner(message: &Message) -> bool {
//     // Get the user ID
//     let user_id = message.from().as_ref().expect("Message has no user").id;

//     user_id == teloxide::prelude::UserId(TELEGRAM_OWNER_ID)
// }

pub fn parse_argument(text: &str) -> State {
    match text.split_whitespace().nth(1) {
        Some(arg) => match arg {
            "enable" | "true" => State::Enable,
            "disable" | "false" => State::Disable,
            "toggle" => State::Toggle,
            _ => State::Toggle,
        },
        None => State::Toggle,
    }
}
