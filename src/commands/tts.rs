use crate::utils::openai::tts;
use crate::utils::openai::Voice;
use lambda_http::{Body, Response};
use lambda_runtime::Error;
use teloxide::payloads::SendVoiceSetters;
use teloxide::types::ChatAction;
use teloxide::types::InputFile;
use teloxide::{payloads::SendMessageSetters, requests::Requester, Bot};
use tracing::error;

pub async fn handle_tts_command(
    bot: Bot,
    message: teloxide::types::Message,
) -> Result<Response<Body>, Error> {
    let text = message.text().unwrap();

    // CHECK FOR REPLY (WE NEED A TEXT INPUT)
    // IF THERE IS NO REPLY, USE THE TEXT FROM THE COMMAND
    let tts_text;
    if let Some(reply) = message.reply_to_message() {
        if let Some(text) = reply.text() {
            tts_text = text;
        } else {
            tts_text = "";
        }
    } else {
        // Get the text from the command
        tts_text = text
            .trim_start_matches("/tts")
            .trim()
            .trim_start_matches("/tts@duck_transcriber_bot")
            .trim();
    }

    // If the text is empty, send a message to the user
    if tts_text.is_empty() {
        bot.send_message(
            message.chat.id,
            "Please provide some text to generate a voice message.",
        )
        .reply_to_message_id(message.id)
        .disable_web_page_preview(true)
        .allow_sending_without_reply(true)
        .await?;
        return Ok(Response::builder()
            .status(200)
            .body(Body::Text("No text provided".into()))
            .unwrap());
    }

    // Send "recording voice message" action to user
    bot.send_chat_action(message.chat.id, ChatAction::RecordVoice)
        .await?;

    // random voice using rand
    let random_voice = match rand::random::<u8>() % 6 {
        0 => Voice::Alloy,
        1 => Voice::Echo,
        2 => Voice::Fable,
        3 => Voice::Onyx,
        4 => Voice::Nova,
        5 => Voice::Shimmer,
        _ => Voice::Alloy,
    };

    let voice = tts(tts_text.to_string(), random_voice).await;

    match voice {
        Ok(voice) => {
            // Send the voice message to the user
            bot.send_voice(message.chat.id, InputFile::memory(voice))
                .caption(format!("Voice: {}", tts_text))
                .reply_to_message_id(message.id)
                .await?;
        }
        Err(e) => {
            error!("Failed to generate voice: {}", e);
            bot.send_message(
                message.chat.id,
                format!("Failed to generate voice. Please try again later. ({e})"),
            )
            .reply_to_message_id(message.id)
            .disable_web_page_preview(true)
            .allow_sending_without_reply(true)
            .await?;

            return Ok(Response::builder()
                .status(200)
                .body(Body::Text(format!("Failed to generate voice: {e}")))
                .unwrap());
        }
    }

    Ok(Response::builder()
        .status(200)
        .body(Body::Text("OK".into()))
        .unwrap())
}
