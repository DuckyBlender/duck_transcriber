use std::env;

use log::info;
use reqwest::multipart;
use teloxide::{net::Download, prelude::*};

use crate::structs::OpenAIResponse;

mod structs;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    // Initialize the logger
    env::set_var("RUST_LOG", "info");
    pretty_env_logger::init();
    log::info!("Starting throw dice bot...");

    let bot = Bot::from_env();

    teloxide::repl(bot, handle_message).await;
}

async fn handle_message(bot: Bot, msg: Message) -> Result<(), teloxide::RequestError> {
    // Check if the message is a voice message
    if let Some(voice) = msg.voice() {
        let voice = voice.clone();
        info!("Received voice message: {:?}", voice);
        // Download the voice message
        let file = bot.get_file(voice.file.id).send().await?;
        let file_path = file.path;
        let mut buf: Vec<u8> = Vec::new();
        bot.download_file(&file_path, &mut buf).await?;

        // Send the voice message to openai whisper
        let client = reqwest::Client::new();

        let api_key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");

        let part = multipart::Part::bytes(buf)
            .file_name("audio.ogg")
            .mime_str("audio/ogg")
            .unwrap();

        let form = multipart::Form::new()
            .part("file", part)
            .text("timestamp_granularities[]", "segment")
            .text("model", "whisper-1")
            .text("response_format", "verbose_json");

        let res = client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .bearer_auth(api_key)
            .multipart(form)
            .send()
            .await
            .unwrap();

        // Deserialize the response into a struct
        let response: OpenAIResponse = res.json().await.unwrap();

        let mut full_text = String::new();
        for segment in response.segments {
            if segment.no_speech_prob > 0.6 && segment.avg_logprob < -0.4 {
                continue;
            }
            info!("Segment (AVG_LOGPROB: {:.2} NO_SPEECH_PROB: ({:.2})): {}", segment.avg_logprob, segment.no_speech_prob, segment.text);
            full_text.push_str(&segment.text);
        }

        if full_text.is_empty() {
            full_text = "<no text>".to_string();
        }

        // Send the response to the user
        bot.send_message(msg.chat.id, full_text)
            .reply_to_message_id(msg.id)
            .disable_web_page_preview(true)
            .await?;
    }
    Ok(())
}