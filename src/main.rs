use std::env;

use log::info;
use reqwest::multipart;
use teloxide::{net::Download, prelude::*};

use crate::structs::OpenAIResponse;

mod structs;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
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
            .file_name("audio.mp3")
            .mime_str("audio/mp3")
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

        // Send the response to the user
        bot.send_message(msg.chat.id, response.text)
            .reply_to_message_id(msg.id)
            .disable_web_page_preview(true)
            .await?;
    }
    Ok(())
}
