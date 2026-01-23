use chrono::Local;
use dotenvy::dotenv;
use log::{debug, error, info, warn};
use mime::Mime;
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::types::{Message, ParseMode, ReactionType};
use teloxide::utils::command::BotCommands;
use teloxide::{prelude::*, utils::html};
use types::{
    AudioAction, AudioFileInfo, BotCommand, SummarizeMethod, TaskType, TranscriptionError,
};
use utils::{pretty_model_name, safe_send, start_typing_indicator};

mod database;
mod summarize;
mod transcribe;
mod types;
mod utils;

use database::Database;

const RATE_LIMIT_PER_MINUTE: u32 = 5;
const RATE_LIMIT_PER_HOUR: u32 = 30;
const MAX_DURATION: u32 = 30;
const MAX_FILE_SIZE: u32 = 20;

pub const BASE_URL: &str = "https://api.groq.com/openai/v1";

#[tokio::main]
async fn main() {
    dotenv().ok();
    setup_logging().expect("Failed to setup logging");

    info!("Starting duck_transcriber bot");

    let api_keys = utils::get_api_keys();
    if api_keys.is_empty() {
        panic!("No API keys configured. Set GROQ_API_KEY environment variable.");
    }

    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set!"));

    let db = match Database::connect().await {
        Ok(db) => {
            if let Err(e) = db.init().await {
                error!("Failed to initialize database: {e}");
                panic!("Failed to initialize database");
            }
            Arc::new(db)
        }
        Err(e) => {
            error!("Failed to connect to database: {e}");
            panic!("Failed to connect to database");
        }
    };

    let res = bot.set_my_commands(BotCommand::bot_commands()).await;
    if let Err(e) = res {
        warn!("Failed to set commands: {e:?}");
    }

    let db_cleanup = db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Err(e) = db_cleanup.cleanup_expired().await {
                error!("Failed to cleanup expired entries: {e}");
            }
        }
    });

    let handler = dptree::entry().branch(Update::filter_message().endpoint(handle_message));

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![db])
        .enable_ctrlc_handler()
        .build();

    info!("Bot started successfully");
    dispatcher.dispatch().await;
}

async fn handle_message(
    bot: Bot,
    message: Message,
    db: Arc<Database>,
) -> Result<(), teloxide::RequestError> {
    let user_id = match message.from.as_ref() {
        Some(user) => user.id,
        None => {
            debug!("Received message without user info");
            return Ok(());
        }
    };

    let bot_username = match bot.get_me().await {
        Ok(me) => me.username.clone().unwrap_or_default(),
        Err(e) => {
            warn!("Failed to get bot info: {e:?}");
            String::new()
        }
    };

    if let Some(text) = message.text()
        && let Ok(command) = BotCommand::parse(text, &bot_username)
    {
        handle_command(&bot, &message, command, &db, user_id).await;
        return Ok(());
    }

    if let Some(caption) = message.caption()
        && let Ok(command) = BotCommand::parse(caption, &bot_username)
    {
        handle_command(&bot, &message, command, &db, user_id).await;
        return Ok(());
    }

    let is_dm = message.chat.is_private();
    let should_auto_transcribe = message.voice().is_some()
        || message.video_note().is_some()
        || (is_dm && (message.video().is_some() || message.audio().is_some()));

    if should_auto_transcribe {
        handle_audio_message(&message, &message, &bot, &db, TaskType::Transcribe, user_id).await;
    }

    Ok(())
}

async fn handle_command(
    bot: &Bot,
    message: &Message,
    command: BotCommand,
    db: &Arc<Database>,
    user_id: teloxide::types::UserId,
) {
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
            handle_audio_command(
                bot,
                message,
                AudioAction::Transcribe,
                "Reply to an audio message or video note to transcribe it.",
                db,
                user_id,
            )
            .await;
        }
        BotCommand::Translate => {
            handle_audio_command(
                bot,
                message,
                AudioAction::Translate,
                "Reply to an audio message or video note to translate it.",
                db,
                user_id,
            )
            .await;
        }
        BotCommand::Summarize => {
            handle_audio_command(
                bot,
                message,
                AudioAction::Summarize(SummarizeMethod::Default),
                "Reply to an audio message or video note to summarize it.",
                db,
                user_id,
            )
            .await;
        }
        BotCommand::Caveman => {
            handle_audio_command(
                bot,
                message,
                AudioAction::Summarize(SummarizeMethod::Caveman),
                "Reply to an audio message or video note to summarize it like a caveman.",
                db,
                user_id,
            )
            .await;
        }
        BotCommand::Privacy => {
            let privacy_policy = format!(
                "<b>Privacy Policy:</b>\n\n\
                • Bot is open source: https://github.com/DuckyBlender/duck_transcriber\n\
                • Bot caches: unique file id → transcription/translation/summary\n\
                • Nothing else is stored, not even in logs\n\
                • Transcription cache is cleared after 7 days\n\
                • Summary cache is cleared after 1 day\n\
                • Join @sussy_announcements for support/questions\n\
                • No guarantees about model accuracy or reliability\n\
                • Uses {} from GroqCloud for instant transcription\n\
                • Uses {} from GroqCloud for instant translation\n\
                • Uses {} from GroqCloud for instant summarization\n\
                • <b>GroqCloud Privacy:</b> Uses Global ZDR (Zero Day Retention) active. No data is stored on GroqCloud servers.",
                pretty_model_name(transcribe::TRANSCRIPTION_MODEL),
                pretty_model_name(transcribe::TRANSLATION_MODEL),
                pretty_model_name(summarize::SUMMARIZATION_MODEL)
            );
            safe_send(
                bot,
                message,
                Some(&privacy_policy),
                Some(ParseMode::Html),
                None,
            )
            .await;
        }
    }
}

async fn handle_audio_command(
    bot: &Bot,
    message: &Message,
    action: AudioAction,
    help_text: &str,
    db: &Arc<Database>,
    user_id: teloxide::types::UserId,
) {
    let target_message = if has_audio_content(message) {
        message
    } else if let Some(reply) = message.reply_to_message() {
        if has_audio_content(reply) {
            reply
        } else {
            safe_send(bot, message, Some(help_text), None, None).await;
            return;
        }
    } else {
        safe_send(bot, message, Some(help_text), None, None).await;
        return;
    };

    match action {
        AudioAction::Transcribe | AudioAction::Translate => {
            let task_type = action.task_type();
            handle_audio_message(target_message, message, bot, db, task_type, user_id).await;
        }
        AudioAction::Summarize(method) => {
            let task_type = action.task_type();
            handle_summarization(target_message, message, method, task_type, bot, db, user_id)
                .await;
        }
    }
}

fn has_audio_content(message: &Message) -> bool {
    message.voice().is_some()
        || message.video_note().is_some()
        || message.video().is_some()
        || message.audio().is_some()
}

async fn setup_audio_processing(message: &Message) -> Option<AudioFileInfo> {
    match AudioFileInfo::from_message(message) {
        Some(info) => {
            info!("Received audio message with duration: {}s", info.duration);
            Some(info)
        }
        None => {
            error!("No audio content found in message");
            None
        }
    }
}

fn validate_file_size(audio_info: &AudioFileInfo) -> Result<(), String> {
    if audio_info.size > MAX_FILE_SIZE * 1024 * 1024 {
        warn!("File is larger than {MAX_FILE_SIZE}MB");
        return Err(format!(
            "File can't be larger than {MAX_FILE_SIZE}MB (is {:.2}MB)",
            audio_info.size as f64 / 1024.0 / 1024.0
        ));
    }
    Ok(())
}

async fn handle_audio_message(
    audio_source_message: &Message,
    reply_context: &Message,
    bot: &Bot,
    db: &Arc<Database>,
    task_type: TaskType,
    user_id: teloxide::types::UserId,
) {
    let audio_info = match setup_audio_processing(audio_source_message).await {
        Some(info) => info,
        None => return,
    };

    match db
        .check_rate_limit(user_id, RATE_LIMIT_PER_MINUTE, RATE_LIMIT_PER_HOUR)
        .await
    {
        Ok(false) => {
            warn!("Rate limit exceeded for user {}", user_id);
            if let Err(err) = bot
                .set_message_reaction(reply_context.chat.id, reply_context.id)
                .reaction([ReactionType::Emoji {
                    emoji: "🙊".to_string(),
                }])
                .await
            {
                error!("Failed to set reaction: {err:?}");
            }
            return;
        }
        Err(e) => {
            error!("Failed to check rate limit: {e}");
        }
        _ => {}
    }

    if let Ok(Some(transcription)) = db
        .get_cached(&audio_info.unique_id.to_string(), &task_type)
        .await
    {
        info!(
            "Transcription found in cache for unique_file_id: {}",
            audio_info.unique_id
        );
        safe_send(
            bot,
            reply_context,
            Some(&transcription),
            None,
            Some(task_type),
        )
        .await;
        return;
    }

    if let Err(error_msg) = validate_file_size(&audio_info) {
        safe_send(bot, reply_context, Some(&error_msg), None, None).await;
        return;
    }

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
        return;
    }

    let typing_guard = start_typing_indicator(bot.clone(), reply_context.chat.id);

    let (audio_bytes, mime, duration) = match download_audio(bot, &audio_info).await {
        Ok(res) => res,
        Err(e) => {
            error!("Failed to download audio: {e:?}");
            safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None).await;
            return;
        }
    };

    info!("Transcribing audio! Duration: {duration} | Mime: {mime:?}");
    let now = std::time::Instant::now();
    let transcription = match transcribe::transcribe(&task_type, audio_bytes, mime).await {
        Ok(transcription) => transcription,
        Err(e) => match e {
            TranscriptionError::RateLimitReached => {
                warn!("Rate limit reached for message");
                if let Err(err) = bot
                    .set_message_reaction(reply_context.chat.id, reply_context.id)
                    .reaction([ReactionType::Emoji {
                        emoji: "🙊".to_string(),
                    }])
                    .await
                {
                    error!("Failed to set reaction: {err:?}");
                }
                return;
            }
            _ => {
                warn!("Failed to transcribe audio: {e}");
                safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None).await;
                return;
            }
        },
    };
    info!("Transcribed audio in {}ms", now.elapsed().as_millis());

    let transcription = transcription
        .unwrap_or("<no text>".to_string())
        .trim()
        .to_string();

    drop(typing_guard);

    safe_send(
        bot,
        reply_context,
        Some(&transcription),
        None,
        Some(task_type),
    )
    .await;

    if let Err(e) = db
        .set_cached(
            &audio_info.unique_id.to_string(),
            &task_type,
            &transcription,
        )
        .await
    {
        error!("Failed to cache transcription: {e}");
    }

    if let Err(e) = db.record_usage(user_id).await {
        error!("Failed to record usage: {e}");
    }
}

async fn handle_summarization(
    audio_source_message: &Message,
    reply_context: &Message,
    method: SummarizeMethod,
    task_type: TaskType,
    bot: &Bot,
    db: &Arc<Database>,
    user_id: teloxide::types::UserId,
) {
    let audio_info = match setup_audio_processing(audio_source_message).await {
        Some(info) => {
            info!(
                "Received audio message for summarization with duration: {}s",
                info.duration
            );
            info
        }
        None => return,
    };

    match db
        .check_rate_limit(user_id, RATE_LIMIT_PER_MINUTE, RATE_LIMIT_PER_HOUR)
        .await
    {
        Ok(false) => {
            warn!("Rate limit exceeded for user {}", user_id);
            if let Err(err) = bot
                .set_message_reaction(reply_context.chat.id, reply_context.id)
                .reaction([ReactionType::Emoji {
                    emoji: "🙊".to_string(),
                }])
                .await
            {
                error!("Failed to set reaction: {err:?}");
            }
            return;
        }
        Err(e) => {
            error!("Failed to check rate limit: {e}");
        }
        _ => {}
    }

    if let Ok(Some(cached_summary)) = db
        .get_cached(&audio_info.unique_id.to_string(), &task_type)
        .await
    {
        info!(
            "Summary found in cache for unique_file_id: {}",
            audio_info.unique_id
        );
        let formatted_summary = format!("<i>{}</i>", html::escape(&cached_summary));
        safe_send(
            bot,
            reply_context,
            Some(&formatted_summary),
            Some(ParseMode::Html),
            Some(TaskType::Summarize),
        )
        .await;
        return;
    }

    let translation = match db
        .get_cached(&audio_info.unique_id.to_string(), &TaskType::Translate)
        .await
    {
        Ok(Some(translation)) => {
            info!(
                "Translation found in cache for unique_file_id: {}",
                audio_info.unique_id
            );
            translation
        }
        _ => {
            if let Err(error_msg) = validate_file_size(&audio_info) {
                safe_send(bot, reply_context, Some(&error_msg), None, None).await;
                return;
            }

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
                return;
            }

            let res = download_audio(bot, &audio_info).await;
            let (audio_bytes, mime, _) = match res {
                Ok(res) => res,
                Err(e) => {
                    error!("Failed to download audio: {e:?}");
                    safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None).await;
                    return;
                }
            };

            match transcribe::transcribe(&TaskType::Translate, audio_bytes, mime).await {
                Ok(Some(translation)) => {
                    info!(
                        "Saving translation to cache with unique_file_id: {}",
                        audio_info.unique_id
                    );

                    if let Err(e) = db
                        .set_cached(
                            &audio_info.unique_id.to_string(),
                            &TaskType::Translate,
                            &translation,
                        )
                        .await
                    {
                        error!("Failed to cache translation: {e}");
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
                    return;
                }
                Err(e) => {
                    safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None).await;
                    return;
                }
            }
        }
    };

    let summary = match summarize::summarize(&translation, method).await {
        Ok(summary) => summary,
        Err(e) => match e {
            TranscriptionError::RateLimitReached => {
                warn!("Rate limit reached for summarization");
                if let Err(err) = bot
                    .set_message_reaction(reply_context.chat.id, reply_context.id)
                    .reaction([ReactionType::Emoji {
                        emoji: "🙊".to_string(),
                    }])
                    .await
                {
                    error!("Failed to set reaction: {err:?}");
                }
                return;
            }
            _ => {
                safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None).await;
                return;
            }
        },
    };

    if let Err(e) = db
        .set_cached(&audio_info.unique_id.to_string(), &task_type, &summary)
        .await
    {
        error!("Failed to cache summary: {e}");
    }

    let formatted_summary = format!("<i>{}</i>", html::escape(&summary));

    safe_send(
        bot,
        reply_context,
        Some(&formatted_summary),
        Some(ParseMode::Html),
        Some(TaskType::Summarize),
    )
    .await;

    if let Err(e) = db.record_usage(user_id).await {
        error!("Failed to record usage: {e}");
    }
}

pub async fn download_audio(
    bot: &Bot,
    audio_info: &AudioFileInfo,
) -> Result<(Vec<u8>, Mime, u32), String> {
    let file = bot
        .get_file(audio_info.file_id.clone())
        .await
        .map_err(|e| format!("Failed to get file: {e}"))?;

    info!(
        "Checking file size: {} bytes ({:.2}MB)",
        file.size,
        file.size as f64 / 1024.0 / 1024.0
    );

    if file.size > MAX_FILE_SIZE * 1024 * 1024 {
        return Err(format!(
            "File can't be larger than {MAX_FILE_SIZE}MB (is {:.2}MB)",
            file.size as f64 / 1024.0 / 1024.0
        ));
    }

    let mut audio_bytes = Vec::new();
    bot.download_file(&file.path, &mut audio_bytes)
        .await
        .map_err(|e| format!("Failed to download file: {e}"))?;

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

fn setup_logging() -> Result<(), fern::InitError> {
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("bot.log")?;

    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}][{}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                message
            ))
        })
        .level(log::LevelFilter::Info)
        .chain(std::io::stdout())
        .chain(log_file)
        .apply()?;

    Ok(())
}
