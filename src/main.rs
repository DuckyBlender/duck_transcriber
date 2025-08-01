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
use summarize::SummarizeMethod;
use teloxide::types::{FileId, FileUniqueId, Message};
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
    #[command(description = "summarize the replied audio message like a caveman")]
    Caveman,
    #[command(description = "show privacy policy")]
    Privacy,
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
        // Handle commands
        if let Some(text) = message.text() {
            if let Ok(command) = BotCommand::parse(text, bot.get_me().await.unwrap().username()) {
                return handle_command(bot, &message, command, dynamodb).await;
            }
        }

        // Handle audio messages and video notes
        if message.voice().is_some() || message.video_note().is_some() {
            return handle_audio_message(&message, bot, dynamodb, TaskType::Transcribe).await;
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
                if has_audio_content(reply) {
                    return handle_audio_message(reply, bot, dynamodb, TaskType::Translate).await;
                }
            }
            safe_send(
                bot,
                message,
                Some("Reply to an audio message or video note to translate it."),
                None,
            )
            .await;
        }
        BotCommand::Transcribe => {
            if let Some(reply) = message.reply_to_message() {
                if has_audio_content(reply) {
                    return handle_audio_message(reply, bot, dynamodb, TaskType::Transcribe).await;
                }
            }
            safe_send(
                bot,
                message,
                Some("Reply to an audio message or video note to transcribe it."),
                None,
            )
            .await;
        }
        BotCommand::Summarize => {
            if let Some(reply) = message.reply_to_message() {
                if has_audio_content(reply) {
                    return handle_summarization(reply, SummarizeMethod::Default, bot, dynamodb)
                        .await;
                }
            }
            safe_send(
                bot,
                message,
                Some("Reply to an audio message or video note to summarize it."),
                None,
            )
            .await;
        }
        BotCommand::Caveman => {
            if let Some(reply) = message.reply_to_message() {
                if has_audio_content(reply) {
                    return handle_summarization(reply, SummarizeMethod::Caveman, bot, dynamodb)
                        .await;
                }
            }
            safe_send(
                bot,
                message,
                Some("Reply to an audio message or video note to summarize it like a caveman."),
                None,
            )
            .await;
        }
        BotCommand::Privacy => {
            let privacy_policy = "Privacy Policy:\n\
            - Bot is open source: https://github.com/DuckyBlender/duck_transcriber\n\
            - Bot caches: unique file id → transcription/translation\n\
            - Nothing else is stored, not even in logs\n\
            - Cache is cleared after 7 days\n\
            - Contact @duckyblender for questions";
            safe_send(bot, message, Some(privacy_policy), None).await;
        }
    }

    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

#[derive(Debug)]
struct AudioFileInfo {
    file_id: FileId,
    unique_id: FileUniqueId,
    duration: u32,
    size: u32,
}

impl AudioFileInfo {
    fn from_message(message: &Message) -> Option<Self> {
        if let Some(voice) = message.voice() {
            return Some(Self {
                file_id: voice.file.id.clone(),
                unique_id: voice.file.unique_id.clone(),
                duration: voice.duration.seconds(),
                size: voice.file.size,
            });
        }
        if let Some(video_note) = message.video_note() {
            return Some(Self {
                file_id: video_note.file.id.clone(),
                unique_id: video_note.file.unique_id.clone(),
                duration: video_note.duration.seconds(),
                size: video_note.file.size,
            });
        }
        if let Some(video) = message.video() {
            return Some(Self {
                file_id: video.file.id.clone(),
                unique_id: video.file.unique_id.clone(),
                duration: video.duration.seconds(),
                size: video.file.size,
            });
        }
        if let Some(audio) = message.audio() {
            return Some(Self {
                file_id: audio.file.id.clone(),
                unique_id: audio.file.unique_id.clone(),
                duration: audio.duration.seconds(),
                size: audio.file.size,
            });
        }
        None
    }
}

fn has_audio_content(message: &Message) -> bool {
    message.voice().is_some()
        || message.video_note().is_some()
        || message.video().is_some()
        || message.audio().is_some()
}

async fn handle_audio_message(
    message: &Message,
    bot: &Bot,
    dynamodb: &aws_sdk_dynamodb::Client,
    task_type: TaskType,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    // Send "typing" indicator
    debug!("Sending typing indicator");
    if let Err(e) = bot
        .send_chat_action(message.chat.id, ChatAction::Typing)
        .await
    {
        warn!("Failed to send typing indicator: {e:?}");
    }

    let audio_info = match AudioFileInfo::from_message(message) {
        Some(info) => {
            info!("Received audio message with duration: {}s", info.duration);
            info
        }
        None => {
            error!("No audio content found in message");
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
    };

    // Get the transcription from DynamoDB
    match dynamodb::get_item(dynamodb, &audio_info.unique_id, &task_type).await {
        Ok(ItemReturnInfo::Text(transcription)) => {
            info!(
                "Transcription found in DynamoDB for unique_file_id: {}",
                audio_info.unique_id
            );
            safe_send(bot, message, Some(&transcription), None).await;
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
    if audio_info.size > MAX_FILE_SIZE * 1024 * 1024 {
        warn!("File is larger than {MAX_FILE_SIZE}MB");
        safe_send(
            bot,
            message,
            Some(&format!(
                "File can't be larger than {MAX_FILE_SIZE}MB (is {}MB)",
                audio_info.size / 1024 / 1024
            )),
            None,
        )
        .await;
        return Ok(lambda_http::Response::builder()
            .status(200)
            .body(String::new())
            .unwrap());
    }

    // Download the audio file
    let (audio_bytes, mime, duration) = match download_audio(bot, &audio_info).await {
        Ok(res) => res,
        Err(e) => {
            error!("Failed to download audio: {e:?}");
            safe_send(bot, message, Some(&format!("Error: {e}")), None).await;
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
    };

    // If the duration is above MAX_DURATION
    if duration > MAX_DURATION * 60 {
        warn!("The audio message is above {MAX_DURATION} minutes!");
        safe_send(
            bot,
            message,
            Some(&format!("Duration is above {MAX_DURATION} minutes")),
            None,
        )
        .await;
        return Ok(lambda_http::Response::builder()
            .status(200)
            .body(String::new())
            .unwrap());
    }

    // Transcribe the message
    info!(
        "Transcribing audio! Duration: {duration} | Mime: {mime:?}"
    );
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
            safe_send(bot, message, Some(&format!("Error: {e}")), None).await;
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
    };
    info!("Transcribed audio in {}ms", now.elapsed().as_millis());

    let transcription = transcription
        .unwrap_or("<no text>".to_string())
        .trim()
        .to_string();

    // Send the transcription to the user
    safe_send(bot, message, Some(&transcription), None).await;

    // Save the transcription to DynamoDB
    let item = dynamodb::DBItem {
        text: transcription.clone(),
        unique_file_id: audio_info.unique_id.to_string(),
        task_type: task_type.to_string(),
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

    Ok(lambda_http::Response::builder()
        .status(200)
        .body(String::new())
        .unwrap())
}

async fn handle_summarization(
    message: &Message,
    method: SummarizeMethod,
    bot: &Bot,
    dynamodb: &aws_sdk_dynamodb::Client,
) -> Result<lambda_http::Response<String>, lambda_http::Error> {
    // Send "typing" indicator
    debug!("Sending typing indicator");
    if let Err(e) = bot
        .send_chat_action(message.chat.id, ChatAction::Typing)
        .await
    {
        warn!("Failed to send typing indicator: {e:?}");
    }

    let audio_info = match AudioFileInfo::from_message(message) {
        Some(info) => {
            info!(
                "Received audio message for summarization with duration: {}s",
                info.duration
            );
            info
        }
        None => {
            error!("No audio content found in message");
            return Ok(lambda_http::Response::builder()
                .status(200)
                .body(String::new())
                .unwrap());
        }
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
                if audio_info.size > MAX_FILE_SIZE * 1024 * 1024 {
                    warn!("File is larger than {MAX_FILE_SIZE}MB");
                    safe_send(
                        bot,
                        message,
                        Some(&format!(
                            "File can't be larger than {MAX_FILE_SIZE}MB (is {}MB)",
                            audio_info.size / 1024 / 1024
                        )),
                        None,
                    )
                    .await;
                    return Ok(lambda_http::Response::builder()
                        .status(200)
                        .body(String::new())
                        .unwrap());
                }
                let res = download_audio(bot, &audio_info).await;
                let (audio_bytes, mime, _) = match res {
                    Ok(res) => res,
                    Err(e) => {
                        error!("Failed to download audio: {e:?}");
                        safe_send(bot, message, Some(&format!("Error: {e}")), None).await;
                        return Ok(lambda_http::Response::builder()
                            .status(200)
                            .body(String::new())
                            .unwrap());
                    }
                };

                match transcribe::transcribe(&TaskType::Translate, audio_bytes, mime).await {
                    Ok(Some(translation)) => {
                        // Cache the translation in DynamoDB
                        let item = dynamodb::DBItem {
                            text: translation.clone(),
                            unique_file_id: audio_info.unique_id.to_string(),
                            task_type: TaskType::Translate.to_string(),
                        };

                        match dynamodb::add_item(dynamodb, item).await {
                            Ok(_) => info!("Successfully cached translation in DynamoDB"),
                            Err(e) => error!("Failed to cache translation in DynamoDB: {e:?}"),
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
                        safe_send(bot, message, Some(&format!("Error: {e}")), None).await;
                        return Ok(lambda_http::Response::builder()
                            .status(200)
                            .body(String::new())
                            .unwrap());
                    }
                }
            }
        };

    // Summarize the translation
    let summary = match summarize::summarize(&translation, method).await {
        Ok(summary) => summary,
        Err(e) => {
            safe_send(bot, message, Some(&format!("Error: {e}")), None).await;
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

async fn download_audio(
    bot: &Bot,
    audio_info: &AudioFileInfo,
) -> Result<(Vec<u8>, Mime, u32), Error> {
    // Get the actual file info from Telegram to get the real file size
    let file = bot.get_file(audio_info.file_id.clone()).await?;

    info!(
        "Checking file size: {} bytes ({}MB)",
        file.size,
        file.size / 1024 / 1024
    );

    // Check file size limit - use the actual file size from Telegram API
    if file.size > MAX_FILE_SIZE * 1024 * 1024 {
        warn!(
            "File is larger than {}MB, but during previous check it was smaller: file {}MB vs audio {}MB",
            MAX_FILE_SIZE,
            file.size / 1024 / 1024,
            audio_info.size / 1024 / 1024
        );
        return Err(Error::from(format!(
            "File can't be larger than {MAX_FILE_SIZE}MB (is {}MB)",
            file.size / 1024 / 1024
        )));
    }

    // Download the file
    let mut audio_bytes = Vec::new();
    bot.download_file(&file.path, &mut audio_bytes).await?;

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

    // Remove the temporary input file immediately after output is captured
    drop(temp_input_file);

    if !output.status.success() {
        // Log the command error output (stderr)
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("FFmpeg command failed with Error: {stderr}");
        return Err(Error::from("FFmpeg conversion failed"));
    }

    // Return the FLAC bytes, MIME type, and duration
    Ok((
        output.stdout, // Use stdout directly as our FLAC bytes
        Mime::from_str("audio/flac").unwrap(),
        audio_info.duration,
    ))
}
