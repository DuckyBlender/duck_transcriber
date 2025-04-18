use aws_config::BehaviorVersion;
use aws_config::meta::region::RegionProviderChain;
use core::str;
use dynamodb::ItemReturnInfo;
use lambda_http::{Error, run, service_fn};
use log::LevelFilter;
use log::{debug, error, info, warn};
use mime::Mime;
use std::env;
use std::io::Write;
use std::process::Command;
use std::str::FromStr;
use teloxide::types::Message;
use teloxide::types::UpdateKind;
use teloxide::types::{ChatAction, ParseMode};
use teloxide::utils::command::BotCommands;
use teloxide::utils::markdown::escape;
use teloxide::{net::Download, prelude::*};
use tempfile::NamedTempFile;
use transcribe::TaskType;
use utils::{parse_webhook, safe_send};

mod dynamodb;
mod summarize;
mod transcribe;
mod utils;

const MAX_DURATION: u32 = 30; // in minutes
const MAX_FILE_SIZE: u32 = 20; // in MB (telegram download limit)
// If telegram raises the limit, we can increase this no problem, as we are using ffmpeg to convert to mono 16kHz. Groq filelimit is also much higher than 20MB

pub const BASE_URL: &str = "https://api.groq.com/openai/v1";

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum BotCommand {
    #[command(description = "display this text")]
    Help,
    #[command(description = "welcome message")]
    Start,
    #[command(description = "transcribe the replied audio")]
    Transcribe,
    #[command(description = "transcribe & translate the replied audio file in English.", aliases = ["english", "en"])]
    Translate,
    #[command(description = "summarize the replied audio message")]
    Summarize,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing for logging
    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!("[{}] {}", record.target(), message))
        })
        .level(LevelFilter::Info)
        .chain(std::io::stdout())
        .apply()?;

    // Setup telegram bot (we do it here because this place is a cold start)
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set!"));

    // Setup AWS DynamoDB conn
    let region_provider = RegionProviderChain::default_provider().or_else("eu-central-1");
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    let dynamodb = aws_sdk_dynamodb::Client::new(&config);

    // Set commands
    let res = bot.set_my_commands(BotCommand::bot_commands()).await;

    if let Err(e) = res {
        warn!("Failed to set commands: {:?}", e);
    }

    // Run the Lambda function
    run(service_fn(|req| handler(req, &bot, &dynamodb))).await
}

async fn handler(
    req: lambda_http::Request,
    bot: &Bot,
    dynamodb: &aws_sdk_dynamodb::Client,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    // Parse JSON webhook
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

    match update.kind {
        UpdateKind::Message(message) => {
            // Handle commands
            if let Some(text) = &message.text() {
                if let Ok(command) = BotCommand::parse(text, bot.get_me().await.unwrap().username())
                {
                    return handle_command(bot, &message, command, dynamodb).await;
                }
            }

            // Handle audio messages and video notes
            if message.voice().is_some() || message.video_note().is_some() {
                return handle_audio_message(&message, bot, dynamodb, TaskType::Transcribe).await;
            }

            // Return 200 OK for non-audio messages & non-commands
            Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap())
        }
        _ => {
            debug!("Received non-message update");
            Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap())
        }
    }
}

async fn handle_command(
    bot: &Bot,
    message: &Message,
    command: BotCommand,
    dynamodb: &aws_sdk_dynamodb::Client,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    match command {
        BotCommand::Help => {
            let desc = BotCommand::descriptions().to_string();
            safe_send(bot, message, Some(&desc), None).await;
        }
        BotCommand::Start => {
            safe_send(
                bot,
                message,
                Some("Welcome! Send a voice message or video note to transcribe it. You can also use /help to see all available commands."),
                None
            ).await;
        }
        BotCommand::Translate => {
            if let Some(reply) = message.reply_to_message() {
                if reply.voice().is_some()
                    || reply.video_note().is_some()
                    || reply.video().is_some()
                    || reply.audio().is_some()
                {
                    return handle_audio_message(reply, bot, dynamodb, TaskType::Translate).await;
                }
            } else {
                safe_send(
                    bot,
                    message,
                    Some("Reply to an audio message or video note to translate it."),
                    None,
                )
                .await;
            }
        }
        BotCommand::Transcribe => {
            if let Some(reply) = message.reply_to_message() {
                if reply.voice().is_some()
                    || reply.video_note().is_some()
                    || reply.video().is_some()
                    || reply.audio().is_some()
                {
                    return handle_audio_message(reply, bot, dynamodb, TaskType::Transcribe).await;
                }
            } else {
                safe_send(
                    bot,
                    message,
                    Some("Reply to an audio message or video note to transcribe it."),
                    None,
                )
                .await;
            }
        }
        BotCommand::Summarize => {
            if let Some(reply) = message.reply_to_message() {
                if reply.voice().is_some()
                    || reply.video_note().is_some()
                    || reply.video().is_some()
                    || reply.audio().is_some()
                {
                    return handle_summarization(reply, bot, dynamodb).await;
                }
            } else {
                safe_send(
                    bot,
                    message,
                    Some("Reply to an audio message or video note to summarize it."),
                    None,
                )
                .await;
            }
        }
    }

    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

async fn handle_audio_message(
    message: &Message,
    bot: &Bot,
    dynamodb: &aws_sdk_dynamodb::Client,
    task_type: TaskType,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    let unique_file_id: &String;

    // Send "typing" indicator
    debug!("Sending typing indicator");
    let action = bot
        .send_chat_action(message.chat.id, ChatAction::Typing)
        .await;
    if let Err(e) = action {
        warn!("Failed to send typing indicator: {:?}", e);
    }

    // Check if the message is a voice or video note or just a video
    if let Some(voice) = message.voice() {
        let filemeta = &voice.file;
        unique_file_id = &filemeta.unique_id;
        info!("Received voice message!");
    } else if let Some(video_note) = message.video_note() {
        let filemeta = &video_note.file;
        unique_file_id = &filemeta.unique_id;
        info!("Received video note!");
    } else if let Some(video_file) = message.video() {
        let filemeta = &video_file.file;
        unique_file_id = &filemeta.unique_id;
        info!("Received video message!");
    } else if let Some(audio) = message.audio() {
        let filemeta = &audio.file;
        unique_file_id = &filemeta.unique_id;
        info!("Received audio message!");
    } else {
        unreachable!();
    }

    // Get the transcription from DynamoDB
    let item = dynamodb::get_item(dynamodb, unique_file_id, &task_type).await;
    let transcription_type = if let Ok(transcription) = item {
        match transcription {
            ItemReturnInfo::Text(transcription) => {
                info!(
                    "Transcription found in DynamoDB for unique_file_id: {}",
                    unique_file_id
                );

                // Send the transcription to the user
                safe_send(bot, message, Some(&transcription), None).await;

                return Ok(lambda_http::Response::builder()
                    .status(200)
                    .body(String::new())
                    .unwrap());
            }
            ItemReturnInfo::Exists => {
                info!(
                    "Item exists in DynamoDB for unique_file_id: {} but for other task type",
                    unique_file_id
                );
                ItemReturnInfo::Exists
            }
            ItemReturnInfo::None => {
                info!("No items found for unique_file_id: {}", unique_file_id);
                ItemReturnInfo::None
            }
        }
    } else {
        error!(
            "Failed to get item from DynamoDB: {:?}",
            item.err().unwrap()
        );
        ItemReturnInfo::None // if something happens ignore the db
    };

    // (audio_bytes, mime, duration) = download_audio(&bot, &message).await?;
    let res = download_audio(bot, message).await;
    if let Err(e) = res {
        error!("Failed to download audio: {:?}", e);
        safe_send(bot, message, Some(&format!("error: {e}")), None).await;

        return Ok(lambda_http::Response::builder()
            .status(200)
            .body(String::new())
            .unwrap());
    }

    let (audio_bytes, mime, duration) = res.unwrap();

    // If the duration is above MAX_DURATION
    if duration > MAX_DURATION * 60 {
        warn!("The audio message is above {MAX_DURATION} minutes!");
        safe_send(
            bot,
            message,
            Some(&format!("Duration is above {} minutes", MAX_DURATION * 60)),
            None,
        )
        .await;

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
    let transcription = transcribe::transcribe(&task_type, audio_bytes, mime).await;
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
            safe_send(bot, message, Some(&format!("error: {e}")), None).await;

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

    // Send the transcription to the user
    safe_send(bot, message, Some(&transcription), None).await;

    // Save the transcription to DynamoDB
    let item = dynamodb::DBItem {
        text: transcription.clone(),
        unique_file_id: unique_file_id.clone(),
        task_type: task_type.to_string(),
    };

    info!(
        "Saving transcription to DynamoDB with unique_file_id: {}",
        unique_file_id
    );

    match transcription_type {
        ItemReturnInfo::Exists => {
            info!(
                "Updating DynamoDB table for unique_file_id: {}",
                unique_file_id
            );
            match dynamodb::append_attribute(dynamodb, unique_file_id, &task_type, &transcription)
                .await
            {
                Ok(_) => info!("Successfully updated transcription in DynamoDB"),
                Err(e) => error!("Failed to update transcription in DynamoDB: {:?}", e),
            }
        }
        ItemReturnInfo::None => match dynamodb::add_item(dynamodb, item).await {
            Ok(_) => info!("Successfully saved transcription to DynamoDB"),
            Err(e) => error!("Failed to save transcription to DynamoDB: {:?}", e),
        },
        ItemReturnInfo::Text(_) => {
            unreachable!();
        }
    }

    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

async fn handle_summarization(
    message: &Message,
    bot: &Bot,
    dynamodb: &aws_sdk_dynamodb::Client,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    let unique_file_id: &String;

    // Send "typing" indicator
    debug!("Sending typing indicator");
    let action = bot
        .send_chat_action(message.chat.id, ChatAction::Typing)
        .await;
    if let Err(e) = action {
        warn!("Failed to send typing indicator: {:?}", e);
    }

    // Check if the message is a voice or video note or just a video
    if let Some(voice) = message.voice() {
        let filemeta = &voice.file;
        unique_file_id = &filemeta.unique_id;
        info!("Received voice message!");
    } else if let Some(video_note) = message.video_note() {
        let filemeta = &video_note.file;
        unique_file_id = &filemeta.unique_id;
        info!("Received video note!");
    } else if let Some(video_file) = message.video() {
        let filemeta = &video_file.file;
        unique_file_id = &filemeta.unique_id;
        info!("Received video message!");
    } else if let Some(audio) = message.audio() {
        let filemeta = &audio.file;
        unique_file_id = &filemeta.unique_id;
        info!("Received audio message!");
    } else {
        unreachable!();
    }

    // Try to get the translation from DynamoDB first
    let item = dynamodb::get_item(dynamodb, unique_file_id, &TaskType::Translate).await;
    let translation = match item {
        Ok(ItemReturnInfo::Text(translation)) => {
            info!(
                "Translation found in DynamoDB for unique_file_id: {}",
                unique_file_id
            );
            translation
        }
        _ => {
            // If we don't have a translation, get it first
            let res = download_audio(bot, message).await;
            if let Err(e) = res {
                error!("Failed to download audio: {:?}", e);
                safe_send(bot, message, Some(&format!("error: {e}")), None).await;

                return Ok(lambda_http::Response::builder()
                    .status(200)
                    .body(String::new())
                    .unwrap());
            }

            let (audio_bytes, mime, _) = res.unwrap();
            match transcribe::transcribe(&TaskType::Translate, audio_bytes, mime).await {
                Ok(Some(translation)) => {
                    // Cache the translation in DynamoDB
                    let item = dynamodb::DBItem {
                        text: translation.clone(),
                        unique_file_id: unique_file_id.clone(),
                        task_type: TaskType::Translate.to_string(),
                    };

                    match dynamodb::add_item(dynamodb, item).await {
                        Ok(_) => info!("Successfully cached translation in DynamoDB"),
                        Err(e) => error!("Failed to cache translation in DynamoDB: {:?}", e),
                    }

                    translation
                }
                Ok(None) => {
                    safe_send(bot, message, Some("No text found in audio"), None).await;
                    return Ok(lambda_http::Response::builder()
                        .status(200)
                        .body(String::new())
                        .unwrap());
                }
                Err(e) => {
                    safe_send(bot, message, Some(&format!("error: {e}")), None).await;
                    return Ok(lambda_http::Response::builder()
                        .status(200)
                        .body(String::new())
                        .unwrap());
                }
            }
        }
    };

    // Summarize the translation
    let summary = match summarize::summarize(&translation).await {
        Ok(summary) => summary,
        Err(e) => {
            safe_send(bot, message, Some(&format!("error: {e}")), None).await;
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
    };

    // Format summary in italics and escape markdown
    let formatted_summary = format!("_{}_", escape(&summary));
    // Send the summary to the user
    safe_send(
        bot,
        message,
        Some(&formatted_summary),
        Some(ParseMode::MarkdownV2),
    )
    .await;

    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

async fn download_audio(bot: &Bot, message: &Message) -> Result<(Vec<u8>, Mime, u32), Error> {
    let mut audio_bytes = Vec::new();
    let duration;

    if let Some(voice) = message.voice() {
        let file = bot.get_file(&voice.file.id).await?;
        if file.size > MAX_FILE_SIZE * 1024 * 1024 {
            return Err(Error::from(format!(
                "File can't be larger than {MAX_FILE_SIZE}MB (current size: {}MB)",
                file.size / 1024 / 1024
            )));
        }
        info!(
            "File size: {} bytes ({}MB)",
            file.size,
            file.size / 1024 / 1024
        );
        duration = voice.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else if let Some(video_note) = message.video_note() {
        let file = bot.get_file(&video_note.file.id).await?;
        if file.size > MAX_FILE_SIZE * 1024 * 1024 {
            return Err(Error::from(format!(
                "File can't be larger than {MAX_FILE_SIZE}MB (current size: {}MB)",
                file.size / 1024 / 1024
            )));
        }
        info!(
            "File size: {} bytes ({}MB)",
            file.size,
            file.size / 1024 / 1024
        );
        duration = video_note.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else if let Some(video_file) = message.video() {
        let file = bot.get_file(&video_file.file.id).await?;
        if file.size > MAX_FILE_SIZE * 1024 * 1024 {
            return Err(Error::from(format!(
                "File can't be larger than {MAX_FILE_SIZE}MB (current size: {}MB)",
                file.size / 1024 / 1024
            )));
        }
        info!(
            "File size: {} bytes ({}MB)",
            file.size,
            file.size / 1024 / 1024
        );
        duration = video_file.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else if let Some(audio) = message.audio() {
        let file = bot.get_file(&audio.file.id).await?;
        if file.size > MAX_FILE_SIZE * 1024 * 1024 {
            return Err(Error::from(format!(
                "File can't be larger than {MAX_FILE_SIZE}MB (current size: {}MB)",
                file.size / 1024 / 1024
            )));
        }
        info!(
            "File size: {} bytes ({}MB)",
            file.size,
            file.size / 1024 / 1024
        );
        duration = audio.duration;
        bot.download_file(&file.path, &mut audio_bytes).await?;
    } else {
        return Err(Error::from("Unsupported message type"));
    }

    // Write downloaded bytes to a temporary file
    let mut temp_input_file = NamedTempFile::new()?;
    temp_input_file.write_all(&audio_bytes)?;

    // Run FFmpeg command to convert to FLAC and output to stdout
    let output = Command::new("ffmpeg")
        .arg("-i")
        .arg(temp_input_file.path().to_str().unwrap())
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg("-map")
        .arg("0:a")
        .arg("-c:a")
        .arg("flac")
        .arg("-f") // Format
        .arg("flac")
        .arg("pipe:1") // Output to stdout
        .output()?;

    if !output.status.success() {
        // Log the command error output (stderr)
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("FFmpeg command failed with error: {}", stderr);
        std::fs::remove_file(temp_input_file.path()).ok();
        return Err(Error::from("FFmpeg conversion failed"));
    }

    // Remove the temporary input file
    std::fs::remove_file(temp_input_file.path()).ok();

    // Return the FLAC bytes, MIME type, and duration
    Ok((
        output.stdout, // Use stdout directly as our FLAC bytes
        Mime::from_str("audio/flac").unwrap(),
        duration.seconds(),
    ))
}
