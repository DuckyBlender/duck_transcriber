use aws_config::meta::region::RegionProviderChain;
use aws_config::BehaviorVersion;
use lambda_http::{run, service_fn, Body, Error, Request};
use mime::Mime;
use std::env;
use std::str::FromStr;
use teloxide::types::ChatAction;
use teloxide::types::Message;
use teloxide::types::ReplyParameters;
use teloxide::types::UpdateKind;
use teloxide::utils::command::BotCommands;
use teloxide::{net::Download, prelude::*};
use tracing::{debug, error, info, warn};
use tracing_subscriber::fmt;

mod dynamodb;
mod kms;
mod transcribe;

const MAX_DURATION: u32 = 30; // in minutes
const DEFAULT_DELAY: u64 = 5;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum BotCommand {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "welcome message.")]
    Start,
    #[command(description = "amount of cached transcriptions.", alias = "cached")]
    Cache,
    // #[command(description = "summarize the replied audio file.")]
    // Summarize,
    // #[command(description = "transcribe the replied audio file in English.")]
    // English,
}

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

    // Setup AWS DynamoDB conn
    let region_provider = RegionProviderChain::default_provider().or_else("eu-central-1");
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    let dynamodb = aws_sdk_dynamodb::Client::new(&config);
    let kms = aws_sdk_kms::Client::new(&config);

    // Set commands
    let res = bot.set_my_commands(BotCommand::bot_commands()).await;

    if let Err(e) = res {
        warn!("Failed to set commands: {:?}", e);
    }

    // Run the Lambda function
    run(service_fn(|req| handler(req, &bot, &dynamodb, &kms))).await
}

async fn handler(
    req: lambda_http::Request,
    bot: &Bot,
    dynamodb: &aws_sdk_dynamodb::Client,
    kms: &aws_sdk_kms::Client,
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

    // Handle commands
    if let UpdateKind::Message(message) = &update.kind {
        if let Some(text) = &message.text() {
            if let Ok(command) = BotCommand::parse(text, bot.get_me().await.unwrap().username()) {
                return handle_command(bot.clone(), message, command, dynamodb).await;
            }
        }
    }

    // Handle audio messages
    handle_audio_message(update, bot, dynamodb, kms).await
}

async fn handle_command(
    bot: Bot,
    message: &Message,
    command: BotCommand,
    dynamodb: &aws_sdk_dynamodb::Client,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    match command {
        BotCommand::Help => {
            bot.send_message(message.chat.id, BotCommand::descriptions().to_string())
                .await
                .unwrap();
        }
        BotCommand::Start => {
            bot.send_message(message.chat.id, "Welcome! Send a voice message or video note to transcribe it. You can also use /help to see all available commands. Currently there are no other commands available.")
                .await
                .unwrap();
        }
        BotCommand::Cache => {
            let item_count = dynamodb::get_item_count(dynamodb).await.unwrap();
            bot.send_message(
                message.chat.id,
                format!("There are {} cached transcriptions.", item_count),
            )
            .await
            .unwrap();
        } // BotCommand::Summarize => {
          //     bot.send_message(message.chat.id, "Please reply to an audio message with /summarize to transcribe it.")
          //         .await
          //         .unwrap();
          // }
          // BotCommand::English => {
          //     bot.send_message(message.chat.id, "Please reply to an audio message with /english to transcribe it in English.")
          //         .await
          //         .unwrap();
          // }
    }

    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

async fn handle_audio_message(
    update: Update,
    bot: Bot,
    dynamodb: &aws_sdk_dynamodb::Client,
    kms: &aws_sdk_kms::Client,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    let audio_bytes: Vec<u8>;
    let file_id: &String;
    let mime;
    let duration;

    // Make sure the message is a voice or video note
    let message = match update.kind {
        UpdateKind::Message(message) => {
            if message.voice().is_none() && message.video_note().is_none() {
                debug!("Received non-voice, non-video note message");
                return Ok(lambda_http::Response::builder()
                    .status(200)
                    .body(String::new())
                    .unwrap());
            }
            message
        }
        _ => {
            debug!("Received non-message update");
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
    };

    // Send "typing" indicator
    debug!("Sending typing indicator");
    bot.send_chat_action(message.chat.id, ChatAction::Typing)
        .await
        .unwrap();

    // Check if the message is a voice or video note
    if let Some(voice) = message.voice() {
        let filemeta = &voice.file;
        file_id = &filemeta.unique_id;
        info!("Received voice message!");
    } else if let Some(video_note) = message.video_note() {
        let filemeta = &video_note.file;
        file_id = &filemeta.unique_id;
        info!("Received video note!");
    } else {
        unreachable!();
    }

    // Get the transcription from DynamoDB
    let item = dynamodb::get_item(dynamodb, file_id).await;
    if let Ok(transcription) = item {
        if let Some(transcription) = transcription {
            // Decrypt the blob
            info!("Transcription found in DynamoDB for File ID: {}", file_id);
            let now = std::time::Instant::now();
            let transcription = kms::decrypt_blob(kms, transcription).await.unwrap();
            info!("Decrypted transcription in {}ms", now.elapsed().as_millis());

            let bot_msg = bot
                .send_message(message.chat.id, &transcription)
                .reply_parameters(ReplyParameters::new(message.id))
                .disable_notification(true)
                .await
                .unwrap();

            if transcription == "<no text>" {
                // delete_later = Some(bot_msg);
                // We can't use delete_later here because we need to return early
                delete_message_delay(&bot, &bot_msg, DEFAULT_DELAY).await;
            }

            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
    } else {
        error!("Failed to get item from DynamoDB: {:?}", item);
    }

    (audio_bytes, mime, duration) = download_audio(&bot, &message).await?;

    // If the duration is above MAX_DURATION
    if duration > MAX_DURATION * 60 {
        warn!("The audio message is above {MAX_DURATION} minutes!");
        bot.send_message(
            message.chat.id,
            format!("Duration is above {} minutes", MAX_DURATION * 60),
        )
        .reply_parameters(ReplyParameters::new(message.id))
        .disable_notification(true)
        .await
        .unwrap();

        // we don't want to delete the message
        // Return early if the audio is too long
        return Ok(lambda_http::Response::builder()
            .status(200)
            .body(String::new())
            .unwrap());
    }

    // Transcribe the message
    info!(
        "Transcribing audio! Duration: {} | Mime: {:?}",
        duration, mime
    );
    let now = std::time::Instant::now();
    let transcription = transcribe::transcribe(audio_bytes, mime).await;
    info!("Transcribed audio in {}ms", now.elapsed().as_millis());

    let transcription = match transcription {
        Ok(transcription) => transcription,
        Err(e) => {
            // If there is a rate limit, return NON-200. We want to retry the transcription later.
            if e.starts_with("Rate limit reached.") {
                return Ok(lambda_http::Response::builder()
                    .status(429)
                    .body("Rate limit reached".into())
                    .unwrap());
            }
            warn!("Failed to transcribe audio: {}", e);
            let bot_msg = bot
                .send_message(message.chat.id, format!("ERROR: {e}"))
                .reply_parameters(ReplyParameters::new(message.id))
                .disable_notification(true)
                .await
                .unwrap();

            delete_message_delay(&bot, &bot_msg, DEFAULT_DELAY).await;

            // Return early if transcription failed
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
    };

    // Send the transcription to the user
    let transcription = transcription
        .unwrap_or("<no text>".to_string())
        .trim()
        .to_string();

    bot.send_message(message.chat.id, &transcription)
        .reply_parameters(ReplyParameters::new(message.id))
        .disable_notification(true)
        .await
        .unwrap();

    info!("Encrypting transcription using KMS");
    let now = std::time::Instant::now();
    let encrypted_transcription = kms::encrypt_string(kms, &transcription).await;
    info!("Encrypted transcription in {}ms", now.elapsed().as_millis());

    if let Err(e) = encrypted_transcription {
        error!("Failed to encrypt transcription: {:?}", e);
        let bot_msg = bot
            .send_message(
                message.chat.id,
                format!("ERROR: Failed to encrypt transcription: {e:?}"),
            )
            .reply_parameters(ReplyParameters::new(message.id))
            .disable_notification(true)
            .await
            .unwrap();

        delete_message_delay(&bot, &bot_msg, DEFAULT_DELAY).await;

        return Ok(lambda_http::Response::builder()
            .status(200)
            .body("Failed to encrypt transcription".into())
            .unwrap());
    }

    let encrypted_transcription = encrypted_transcription.unwrap();

    // Save the transcription to DynamoDB
    let item = dynamodb::DBItem {
        transcription: encrypted_transcription,
        file_id: file_id.clone(),
        unix_timestamp: chrono::Utc::now().timestamp(),
    };

    info!(
        "Saving encrypted transcription to DynamoDB with File ID: {}",
        file_id
    );

    match dynamodb::add_item(dynamodb, item).await {
        Ok(_) => info!("Successfully saved encrypted transcription to DynamoDB"),
        Err(e) => error!(
            "Failed to save encrypted transcription to DynamoDB: {:?}",
            e
        ),
    }

    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

async fn download_audio(bot: &Bot, message: &Message) -> Result<(Vec<u8>, Mime, u32), Error> {
    let mut audio_bytes = Vec::new();
    let mime;
    let duration;

    if let Some(audio) = message.audio() {
        let file = bot.get_file(&audio.file.id).await?;
        mime = audio.mime_type.clone().unwrap();
        duration = audio.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else if let Some(voice) = message.voice() {
        let file = bot.get_file(&voice.file.id).await?;
        mime = voice
            .mime_type
            .clone()
            .unwrap_or_else(|| Mime::from_str("audio/ogg").unwrap());
        duration = voice.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else if let Some(video_note) = message.video_note() {
        let file = bot.get_file(&video_note.file.id).await?;
        mime = Mime::from_str("video/mp4").unwrap();
        duration = video_note.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else {
        return Err(Error::from("Unsupported message type"));
    }

    Ok((audio_bytes, mime, duration.seconds()))
}

pub async fn parse_webhook(input: Request) -> Result<Update, Error> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        not => panic!("expected Body::Text(...) got {not:?}"),
    };
    let body_json: Update = serde_json::from_str(body_str)?;
    Ok(body_json)
}

pub async fn delete_message_delay(bot: &Bot, msg: &Message, delay: u64) {
    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
    bot.delete_message(msg.chat.id, msg.id).await.unwrap();
}
