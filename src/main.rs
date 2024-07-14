use lambda_http::{run, service_fn, Body, Error, Request};
use mime::Mime;
use std::env;
use std::io::Write;
use std::str::FromStr;
use std::time::Instant;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::UpdateKind::Message;
use teloxide::{net::Download, requests::Requester, types::Update, Bot};
use tracing::{debug, error, info, warn};
use tracing_subscriber::fmt;

mod transcribe;

const MAX_DURATION: u32 = 30 * 60;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing for logging
    fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .without_time()
        .init();

    // Setup telegram bot (we do it here because this place is a cold start)
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set!"));

    // Set commands
    bot.set_my_commands(vec![])
        .await
        .expect("Failed to set commands");

    // Run the Lambda function
    run(service_fn(|req| handler(req, &bot))).await
}

async fn handler(
    req: lambda_http::Request,
    bot: &Bot,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    // Parse JSON webhook
    let bot = bot.clone();
    let update = match parse_webhook(req).await {
        Ok(message) => message,
        Err(e) => {
            error!("Failed to parse webhook: {:?}", e);
            return Ok(lambda_http::Response::builder()
                .status(400)
                .body("Failed to parse webhook".into())
                .unwrap());
        }
    };

    // Make sure the message is a voice, audio or video note
    let message = match update.kind {
        Message(message) => {
            if message.voice().is_none() && message.video_note().is_none() {
                debug!("Received non-voice, non-audio, non-video note message");
                return Ok(lambda_http::Response::builder()
                    .status(200)
                    .body("".into())
                    .unwrap());
            }
            message
        }
        _ => {
            debug!("Received non-message update");
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body("".into())
                .unwrap());
        }
    };

    // Send "typing" indicator
    debug!("Sending typing indicator");
    bot.send_chat_action(message.chat.id, teloxide::types::ChatAction::Typing)
        .await
        .unwrap();

    let mut audio_bytes: Vec<u8> = Vec::new();
    let mime;
    let duration;

    // Check if the message is a voice, audio or video note
    if let Some(voice) = message.voice() {
        let filemeta = &voice.file;
        info!("Received voice message: {:?}", filemeta);

        let file_id = &voice.file.id;
        let file = bot.get_file(file_id).await.unwrap();
        // default is ogg
        mime = voice
            .mime_type
            .clone()
            .unwrap_or(Mime::from_str("audio/ogg").unwrap());
        duration = voice.duration;
        bot.download_file(&file.path, &mut audio_bytes)
            .await
            .unwrap();
    } else if let Some(video_note) = message.video_note() {
        let filemeta = &video_note.file;
        info!("Received video note message: {:?}", filemeta);
        let file_id = &video_note.file.id;
        let file = bot.get_file(file_id).await.unwrap();
        mime = Mime::from_str("video/mp4").unwrap();
        duration = video_note.duration;
        bot.download_file(&file.path, &mut audio_bytes)
            .await
            .unwrap();
    } else {
        debug!("Received non-voice, non-video note message");
        return Ok(lambda_http::Response::builder()
            .status(200)
            .body("".into())
            .unwrap());
    }

    // If the duration is above MAX_DURATION
    if duration > MAX_DURATION {
        warn!("The audio message is above {MAX_DURATION} seconds!");
        bot.send_message(
            message.chat.id,
            format!("Duration is above {} minutes", MAX_DURATION * 60),
        )
        .reply_to_message_id(message.id)
        .disable_notification(true)
        .await
        .unwrap();

        return Ok(lambda_http::Response::builder()
            .status(200)
            .body("".into())
            .unwrap());
    }

    // Convert the audio
    info!(
        "Converting audio! Duration: {} | Mime: {:?}",
        duration, mime
    );
    let audio_bytes = match ffmpeg_convert(audio_bytes, mime).await {
        Ok(audio_bytes) => audio_bytes,
        Err(e) => {
            error!("Failed to convert audio: {:?}", e);
            let bot_msg = bot
                .send_message(message.chat.id, format!("Failed to convert audio: {:?}", e))
                .disable_web_page_preview(true)
                .disable_notification(true)
                .reply_to_message_id(message.id)
                .await
                .unwrap();
            delete_msg_delay(&bot, &bot_msg, 3).await;
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body("".into())
                .unwrap());
        }
    };
    // After ffmpeg mime is wav
    let mime = Mime::from_str("audio/wav").unwrap();

    // Transcribe the message
    info!("Transcribing audio! Duration: {}", duration);
    let transcription = transcribe::transcribe(audio_bytes, mime).await;

    let transcription = match transcription {
        Ok(transcription) => transcription,
        Err(e) => {
            error!("Failed to transcribe audio: {:?}", e);
            let bot_msg = bot
                .send_message(
                    message.chat.id,
                    format!("Failed to transcribe audio: {:?}", e),
                )
                .disable_web_page_preview(true)
                .disable_notification(true)
                .reply_to_message_id(message.id)
                .await
                .unwrap();

            delete_msg_delay(&bot, &bot_msg, 3).await;
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body("".into())
                .unwrap());
        }
    };

    let transcription = transcription.unwrap_or("<no text>".to_string());

    // Send the transcription to the user
    info!("Transcription: {}", transcription);
    let bot_msg = bot
        .send_message(message.chat.id, transcription.clone())
        .reply_to_message_id(message.id)
        .disable_web_page_preview(true)
        .disable_notification(true)
        .await
        .unwrap();

    if transcription == "<no text>" {
        delete_msg_delay(&bot, &bot_msg, 3).await;
    }

    Ok(lambda_http::Response::builder()
        .status(200)
        .body("".into())
        .unwrap())
}

pub async fn parse_webhook(input: Request) -> Result<Update, Error> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        not => panic!("expected Body::Text(...) got {not:?}"),
    };
    let body_json: Update = serde_json::from_str(body_str)?;
    debug!("Parsed webhook: {:?}", body_json);
    Ok(body_json)
}

pub async fn ffmpeg_convert(bytes: Vec<u8>, mime: Mime) -> Result<Vec<u8>, String> {
    // Convert the audio to wav
    let file_extension = mime.subtype();
    let file_extension = file_extension.as_ref();
    info!(
        "Input bytes: {} | File extension: {}",
        bytes.len(),
        file_extension
    );
    let now = Instant::now();
    let mut command = std::process::Command::new("./ffmpeg");
    command
        .arg("-i")
        .arg("pipe:0")
        .arg("-f")
        .arg(file_extension)
        .arg("-vn")
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg("-f")
        .arg("wav")
        .arg("pipe:1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = command.spawn().map_err(|err| {
        error!("Failed to spawn ffmpeg: {}", err);
        format!("Failed to spawn ffmpeg: {}", err)
    })?;

    let stdin = child.stdin.as_mut().ok_or("Failed to open stdin")?;
    stdin.write_all(&bytes).map_err(|err| {
        error!("Failed to write to stdin: {}", err);
        format!("Failed to write to stdin: {}", err)
    })?;

    let output = child.wait_with_output().map_err(|err| {
        error!("Failed to wait for ffmpeg: {}", err);
        format!("Failed to wait for ffmpeg: {}", err)
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Failed to convert audio to wav. FFmpeg stderr: {}", stderr);
        return Err(format!(
            "Failed to convert audio to wav. FFmpeg stderr: {}",
            stderr
        ));
    }

    if output.stdout.is_empty() {
        error!("FFmpeg produced empty output");
        return Err("FFmpeg produced empty output".to_string());
    }

    info!(
        "Output bytes: {} | Took {:.2}s",
        output.stdout.len(),
        now.elapsed().as_secs()
    );

    info!("FULL OUTPUT: {}", String::from_utf8_lossy(&output.stdout));

    Ok(output.stdout)
}

pub async fn delete_msg_delay(bot: &Bot, msg: &teloxide::types::Message, delay: u64) {
    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
    bot.delete_message(msg.chat.id, msg.id).await.unwrap();
}
