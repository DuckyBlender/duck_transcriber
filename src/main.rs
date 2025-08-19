use aws_config::BehaviorVersion;
use aws_config::meta::region::RegionProviderChain;
use core::str;

use lambda_http::{Error, run, service_fn};
use log::LevelFilter;
use log::{debug, error, info, warn};
use mime::Mime;
use std::env;
use std::str::FromStr;
use teloxide::types::Message;
use teloxide::types::ParseMode;
use teloxide::types::UpdateKind;
use teloxide::utils::command::BotCommands;
use teloxide::utils::markdown::escape;
use teloxide::{net::Download, prelude::*};
use types::{
    AudioAction, AudioFileInfo, BotCommand, DBItem, ItemReturnInfo, SummarizeMethod, TaskType,
};
use utils::{parse_webhook, safe_send, start_typing_indicator};

mod dynamodb;
mod summarize;
mod transcribe;
mod types;
mod utils;

const MAX_DURATION: u32 = 30; // in minutes
const MAX_FILE_SIZE: u32 = 20; // in MB (telegram download limit)

pub const BASE_URL: &str = "https://api.groq.com/openai/v1";

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
        warn!("Failed to set commands: {e:?}");
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
            error!("Failed to parse webhook: {e:?}");
            return Ok(lambda_http::Response::builder()
                .status(400)
                .body("Failed to parse webhook".into())
                .unwrap());
        }
    };

    if let UpdateKind::Message(message) = update.kind {
        // Handle commands in text
        if let Some(text) = message.text()
            && let Ok(command) = BotCommand::parse(text, bot.get_me().await.unwrap().username())
        {
            return handle_command(bot, &message, command, dynamodb).await;
        }

        // Handle commands in caption (when attached to media)
        if let Some(caption) = message.caption()
            && let Ok(command) = BotCommand::parse(caption, bot.get_me().await.unwrap().username())
        {
            return handle_command(bot, &message, command, dynamodb).await;
        }

        // Handle audio messages and video notes (auto-transcribe)
        if message.voice().is_some() || message.video_note().is_some() {
            return handle_audio_message(&message, &message, bot, dynamodb, TaskType::Transcribe)
                .await;
        }
    } else {
        debug!("Received non-message update");
    }

    // Return 200 OK for non-audio messages & non-commands
    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

// Helper to create success response
fn ok_response() -> Result<lambda_http::Response<String>, lambda_http::Error> {
    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

// Generic handler for audio processing commands
async fn handle_audio_command(
    bot: &Bot,
    message: &Message,
    action: AudioAction,
    help_text: &str,
    dynamodb: &aws_sdk_dynamodb::Client,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    // Find target message with audio content
    let target_message = if has_audio_content(message) {
        // Caption command - process current message
        message
    } else if let Some(reply) = message.reply_to_message() {
        if has_audio_content(reply) {
            // Reply command - process replied message
            reply
        } else {
            // No audio content found
            safe_send(bot, message, Some(help_text), None, None).await;
            return ok_response();
        }
    } else {
        // No audio content found
        safe_send(bot, message, Some(help_text), None, None).await;
        return ok_response();
    };

    // Process the audio content
    match action {
        AudioAction::Transcribe(task_type) => {
            // Always reply to the command message, process audio from target_message
            handle_audio_message(target_message, message, bot, dynamodb, task_type).await
        }
        AudioAction::Summarize(method) => {
            // Always reply to the command message, process audio from target_message
            handle_summarization(target_message, message, method, bot, dynamodb).await
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
            safe_send(bot, message, Some(&desc), None, None).await;
        }
        BotCommand::Start => {
            safe_send(
                bot,
                message,
                Some("Welcome! Send a voice message or video note to transcribe it. You can also use /help to see all available commands."),
                None,
                None,
            )
            .await;
        }
        BotCommand::Transcribe => {
            return handle_audio_command(
                bot,
                message,
                AudioAction::Transcribe(TaskType::Transcribe),
                "Reply to an audio message or video note to transcribe it.",
                dynamodb,
            )
            .await;
        }
        BotCommand::Translate => {
            return handle_audio_command(
                bot,
                message,
                AudioAction::Transcribe(TaskType::Translate),
                "Reply to an audio message or video note to translate it.",
                dynamodb,
            )
            .await;
        }
        BotCommand::Summarize => {
            return handle_audio_command(
                bot,
                message,
                AudioAction::Summarize(SummarizeMethod::Default),
                "Reply to an audio message or video note to summarize it.",
                dynamodb,
            )
            .await;
        }
        BotCommand::Caveman => {
            return handle_audio_command(
                bot,
                message,
                AudioAction::Summarize(SummarizeMethod::Caveman),
                "Reply to an audio message or video note to summarize it like a caveman.",
                dynamodb,
            )
            .await;
        }
        BotCommand::Privacy => {
            let privacy_policy = "Privacy Policy:\n\
            - Bot is open source: https://github.com/DuckyBlender/duck_transcriber\n\
            - Bot caches: unique file id → transcription/translation\n\
            - Nothing else is stored, not even in logs\n\
            - Cache is cleared after 7 days\n\
            - Join @sussy_announcements for support/questions\n\
            - No guarantees about model accuracy or reliability\n\
            - Uses Whisper v3 (GroqCloud) for transcription/translation";
            safe_send(bot, message, Some(privacy_policy), None, None).await;
        }
    }

    ok_response()
}

fn has_audio_content(message: &Message) -> bool {
    message.voice().is_some()
        || message.video_note().is_some()
        || message.video().is_some()
        || message.audio().is_some()
}

// Common setup for audio processing
async fn setup_audio_processing(
    message: &Message,
) -> Result<AudioFileInfo, lambda_http::Response<String>> {
    match AudioFileInfo::from_message(message) {
        Some(info) => {
            info!("Received audio message with duration: {}s", info.duration);
            Ok(info)
        }
        None => {
            error!("No audio content found in message");
            Err(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap())
        }
    }
}

// Check file size limits
fn validate_file_size(audio_info: &AudioFileInfo) -> Result<(), String> {
    if audio_info.size > MAX_FILE_SIZE * 1024 * 1024 {
        warn!("File is larger than {MAX_FILE_SIZE}MB");
        return Err(format!(
            "File can't be larger than {MAX_FILE_SIZE}MB (is {}MB)",
            audio_info.size / 1024 / 1024
        ));
    }
    Ok(())
}

async fn handle_audio_message(
    audio_source_message: &Message,
    reply_context: &Message,
    bot: &Bot,
    dynamodb: &aws_sdk_dynamodb::Client,
    task_type: TaskType,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    let audio_info = match setup_audio_processing(audio_source_message).await {
        Ok(info) => info,
        Err(response) => return Ok(response),
    };

    // Get the transcription from DynamoDB
    match dynamodb::get_item(dynamodb, &audio_info.unique_id, &task_type).await {
        Ok(ItemReturnInfo::Text(transcription)) => {
            info!(
                "Transcription found in DynamoDB for unique_file_id: {}",
                audio_info.unique_id
            );
            let label = match task_type {
                TaskType::Transcribe => "transcript",
                TaskType::Translate => "translation",
            };
            safe_send(bot, reply_context, Some(&transcription), None, Some(label)).await;
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
        Ok(ItemReturnInfo::Exists) => {
            info!(
                "Item exists in DynamoDB for unique_file_id: {} but for other task type",
                audio_info.unique_id
            );
        }
        Ok(ItemReturnInfo::None) => {
            info!(
                "No items found for unique_file_id: {}",
                audio_info.unique_id
            );
        }
        Err(e) => {
            error!("Failed to get item from DynamoDB: {e:?}");
        }
    }

    // Check file size limit
    if let Err(error_msg) = validate_file_size(&audio_info) {
        safe_send(bot, reply_context, Some(&error_msg), None, None).await;
        return ok_response();
    }

    // Check duration limit early (before any download)
    if audio_info.duration > MAX_DURATION * 60 {
        warn!("The audio message is above {MAX_DURATION} minutes!");
        safe_send(
            bot,
            reply_context,
            Some(&format!("Duration is above {MAX_DURATION} minutes")),
            None,
            None,
        )
        .await;
        return ok_response();
    }

    // Start typing indicator now that we know we will transcribe (no DynamoDB hit)
    let typing_guard = start_typing_indicator(bot.clone(), reply_context.chat.id);

    // Download the audio file
    let (audio_bytes, mime, duration) = match download_audio(bot, &audio_info).await {
        Ok(res) => res,
        Err(e) => {
            error!("Failed to download audio: {e:?}");
            safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None).await;
            return ok_response();
        }
    };

    // Transcribe the message
    info!("Transcribing audio! Duration: {duration} | Mime: {mime:?}");
    let now = std::time::Instant::now();
    let transcription = match transcribe::transcribe(&task_type, audio_bytes, mime).await {
        Ok(transcription) => transcription,
        Err(e) => {
            if e.starts_with("Rate limit reached.") {
                return Ok(lambda_http::Response::builder()
                    .status(429)
                    .body("Rate limit reached".into())
                    .unwrap());
            }
            warn!("Failed to transcribe audio: {e}");
            safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None).await;
            return ok_response();
        }
    };
    info!("Transcribed audio in {}ms", now.elapsed().as_millis());

    let transcription = transcription
        .unwrap_or("<no text>".to_string())
        .trim()
        .to_string();

    // Stop typing indicator before sending the message
    drop(typing_guard);

    // Send the transcription/translation to the user
    let label = match task_type {
        TaskType::Transcribe => "transcript",
        TaskType::Translate => "translation",
    };
    safe_send(bot, reply_context, Some(&transcription), None, Some(label)).await;

    // Save the transcription to DynamoDB
    let item = DBItem {
        text: transcription.clone(),
        unique_file_id: audio_info.unique_id.to_string(),
        task_type: task_type.to_string(),
        expires_at: (chrono::Utc::now() + chrono::Duration::days(7)).timestamp(),
    };

    info!(
        "Saving transcription to DynamoDB with unique_file_id: {}",
        audio_info.unique_id
    );

    match dynamodb::get_item(dynamodb, &audio_info.unique_id, &task_type).await {
        Ok(ItemReturnInfo::Exists) => {
            info!(
                "Updating DynamoDB table for unique_file_id: {}",
                audio_info.unique_id
            );
            match dynamodb::append_attribute(
                dynamodb,
                &audio_info.unique_id,
                &task_type,
                &transcription,
            )
            .await
            {
                Ok(_) => info!("Successfully updated transcription in DynamoDB"),
                Err(e) => error!("Failed to update transcription in DynamoDB: {e:?}"),
            }
        }
        _ => match dynamodb::add_item(dynamodb, item).await {
            Ok(_) => info!("Successfully saved transcription to DynamoDB"),
            Err(e) => error!("Failed to save transcription to DynamoDB: {e:?}"),
        },
    }

    ok_response()
}

async fn handle_summarization(
    audio_source_message: &Message,
    reply_context: &Message,
    method: SummarizeMethod,
    bot: &Bot,
    dynamodb: &aws_sdk_dynamodb::Client,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    let audio_info = match setup_audio_processing(audio_source_message).await {
        Ok(info) => {
            info!(
                "Received audio message for summarization with duration: {}s",
                info.duration
            );
            info
        }
        Err(response) => return Ok(response),
    };

    // Try to get the translation from DynamoDB first
    let translation =
        match dynamodb::get_item(dynamodb, &audio_info.unique_id, &TaskType::Translate).await {
            Ok(ItemReturnInfo::Text(translation)) => {
                info!(
                    "Translation found in DynamoDB for unique_file_id: {}",
                    audio_info.unique_id
                );
                translation
            }
            _ => {
                // If we don't have a translation, get it first, but first check the file size
                if let Err(error_msg) = validate_file_size(&audio_info) {
                    safe_send(bot, reply_context, Some(&error_msg), None, None).await;
                    return ok_response();
                }

                // Enforce duration limit here as well
                if audio_info.duration > MAX_DURATION * 60 {
                    warn!("The audio message is above {MAX_DURATION} minutes!");
                    safe_send(
                        bot,
                        reply_context,
                        Some(&format!("Duration is above {MAX_DURATION} minutes")),
                        None,
                        None,
                    )
                    .await;
                    return ok_response();
                }
                let res = download_audio(bot, &audio_info).await;
                let (audio_bytes, mime, _) = match res {
                    Ok(res) => res,
                    Err(e) => {
                        error!("Failed to download audio: {e:?}");
                        safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None)
                            .await;
                        return ok_response();
                    }
                };

                match transcribe::transcribe(&TaskType::Translate, audio_bytes, mime).await {
                    Ok(Some(translation)) => {
                        // Cache the translation in DynamoDB
                        let item = DBItem {
                            text: translation.clone(),
                            unique_file_id: audio_info.unique_id.to_string(),
                            task_type: TaskType::Translate.to_string(),
                            expires_at: (chrono::Utc::now() + chrono::Duration::days(7))
                                .timestamp(),
                        };

                        match dynamodb::add_item(dynamodb, item).await {
                            Ok(_) => info!("Successfully cached translation in DynamoDB"),
                            Err(e) => error!("Failed to cache translation in DynamoDB: {e:?}"),
                        }
                        translation
                    }
                    Ok(None) => {
                        safe_send(
                            bot,
                            reply_context,
                            Some("No text found in audio"),
                            None,
                            None,
                        )
                        .await;
                        return ok_response();
                    }
                    Err(e) => {
                        safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None)
                            .await;
                        return ok_response();
                    }
                }
            }
        };

    // Summarize the translation
    let summary = match summarize::summarize(&translation, method).await {
        Ok(summary) => summary,
        Err(e) => {
            safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None).await;
            return ok_response();
        }
    };

    // Format summary in italics and escape markdown
    let formatted_summary = format!("_{}_", escape(&summary));

    // Send the summary to the user
    safe_send(
        bot,
        reply_context,
        Some(&formatted_summary),
        Some(ParseMode::MarkdownV2),
        Some("summarization"),
    )
    .await;

    ok_response()
}

pub async fn download_audio(
    bot: &Bot,
    audio_info: &AudioFileInfo,
) -> Result<(Vec<u8>, Mime, u32), Error> {
    // Get the file metadata from Telegram
    let file = bot.get_file(audio_info.file_id.clone()).await?;

    info!(
        "Checking file size: {} bytes ({}MB)",
        file.size,
        file.size / 1024 / 1024
    );

    if file.size > MAX_FILE_SIZE * 1024 * 1024 {
        return Err(Error::from(format!(
            "File can't be larger than {MAX_FILE_SIZE}MB (is {}MB)",
            file.size / 1024 / 1024
        )));
    }

    // Download file into memory
    let mut audio_bytes = Vec::new();
    bot.download_file(&file.path, &mut audio_bytes).await?;

    // Prefer MIME from Telegram message if provided; otherwise infer from file path
    let mime: Mime = if let Some(m) = audio_info.mime.clone() {
        m
    } else {
        let guessed = mime_guess::from_path(&file.path).first_or_octet_stream();
        let essence = guessed.essence_str();
        Mime::from_str(essence).unwrap_or_else(|_| {
            warn!(
                "Failed to parse guessed MIME '{}' — defaulting to application/octet-stream",
                essence
            );
            Mime::from_str("application/octet-stream").unwrap()
        })
    };

    Ok((audio_bytes, mime, audio_info.duration))
}
