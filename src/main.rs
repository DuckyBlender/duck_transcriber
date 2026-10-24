use chrono::Local;
use dotenvy::dotenv;
use log::{debug, error, info, warn};
use mime::Mime;
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::types::UserId;
use teloxide::types::{Message, ParseMode, ReactionType};
use teloxide::utils::command::BotCommands;
use teloxide::{prelude::*, utils::html};
use types::{AudioAction, AudioFileInfo, BotCommand, SummarizeMethod, TaskType};
use utils::{safe_send, send_transcript, start_typing_indicator};

mod database;
mod llm;
mod phonon;
mod transcribe;
mod types;
mod utils;

use database::{Database, LlmLimits, LlmUsage};

const RATE_LIMIT_PER_MINUTE: u32 = 25;
const RATE_LIMIT_PER_HOUR: u32 = 150;
const MAX_DURATION: u32 = 30;
const MAX_FILE_SIZE: u32 = 20;
const USER_RATE_LIMIT_REACTION: &str = "🙊";
// llama.cpp translations and summaries are slow and share the GPU with
// whisper.cpp, so they get their own, much lower limits.
const LLM_LIMITS: LlmLimits = LlmLimits {
    per_user_hour: 15,
    per_user_day: 50,
    global_hour: 100,
};
// Translating writes out the whole transcript at ~12 tokens/s, about 15 s per
// minute of audio; 10 minutes stays well inside the LLM timeout.
const MAX_TRANSLATE_DURATION: u32 = 10;
// Reacted when the bot as a whole has hit the LLM limit.
const BUSY_REACTION: &str = "😴";
// Reacted instead of replying when whisper.cpp or llama.cpp can't produce a result.
const FAILED_REACTION: &str = "💔";

#[tokio::main]
async fn main() {
    dotenv().ok();
    setup_logging().expect("Failed to setup logging");

    info!("Starting duck_transcriber bot");

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
    user_id: UserId,
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
        BotCommand::Phonon => {
            handle_audio_command(
                bot,
                message,
                AudioAction::Phonon,
                "Reply to an audio message or video note to transcribe it with Phonon-2.5.",
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
                • All cache is in-memory only and cleared on bot restart\n\
                • Transcriptions/translations cache for 7 days; summaries for 1 hour\n\
                • Rate limits use temporary in-memory per-user timestamps\n\
                • Message contents and user details are not stored in logs\n\
                • Join @sussy_announcements for support/questions\n\
                • Transcribes with {} running locally on an RTX 3050 Mobile\n\
                • /phonon transcribes with {} running locally on the CPU\n\
                • Translates and summarizes with {} running locally on the same machine\n\
                • Audio and text are never sent to any third-party service",
                transcribe::WHISPER_MODEL,
                phonon::PHONON_MODEL,
                llm::LLM_MODEL
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
        BotCommand::Limits => {
            let rate_limit_info = format!(
                "<b>📊 Rate Limit Info</b>\n\n\
                <b>All audio commands</b> (per user)\n\
                • <b>Per Minute:</b> {} requests\n\
                • <b>Per Hour:</b> {} requests\n\n\
                <b>/translate, /summarize and /caveman</b>\n\
                • <b>Per User:</b> {} per hour, {} per day\n\
                • <b>Whole Bot:</b> {} per hour\n\
                • <b>/translate:</b> recordings up to {} minutes\n\
                • Results that are already cached don't count\n\n\
                Everything runs on one local GPU, so the AI commands are limited to keep the bot fast for everyone.\n\n\
                <b>Reactions</b>\n\
                • 🙊 You hit your limit\n\
                • 😴 The bot is busy, try again later\n\
                • 💔 Transcription or the AI failed, try again later",
                RATE_LIMIT_PER_MINUTE,
                RATE_LIMIT_PER_HOUR,
                LLM_LIMITS.per_user_hour,
                LLM_LIMITS.per_user_day,
                LLM_LIMITS.global_hour,
                MAX_TRANSLATE_DURATION
            );
            safe_send(
                bot,
                message,
                Some(&rate_limit_info),
                Some(ParseMode::Html),
                None,
            )
            .await;
        }
        BotCommand::Donate => {
            let donation_message = "<b>Support duck_transcriber_bot</b>\n\
                If you would like to help cover hosting costs, you can donate!\n\n\
                <b>Bitcoin</b>\n\
                <code>bc1qqvahwa3we2eymeku5489kpdk29pvddnn8yruzy</code>\n\n\
                <b>Bitcoin Lightning</b>\n\
                <code>duckyblender@strike.me</code>\n\n\
                <b>Ethereum</b> (or any ERC20 token)\n\
                <code>0xd399774D0a969FC2ddF7D0D0f4f165638541408c</code>\n\n\
                <b>Nano</b>\n\
                <code>nano_3ociqkh6taqqu7q7h99oiyuasnkugm7bss87r1r4eph7dym3tmp3cebtosc5</code>\n\n\
                <b>Monero</b>\n\
                <code>49afHkLvAEAXmdCqhWDCYvZiBD942ouwrhxbwto2HXzoGW877umX2aEFcVf8qzpawuFTBo28TK5Kg8sSzSQSXh7D7YoKEeK</code>\n\n\
                Thank you for your support! 🦆";
            safe_send(
                bot,
                message,
                Some(donation_message),
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
    user_id: UserId,
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
        AudioAction::Transcribe | AudioAction::Phonon | AudioAction::Translate => {
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

fn validate_audio_limits(audio_info: &AudioFileInfo) -> Result<(), String> {
    validate_file_size(audio_info)?;

    if audio_info.duration > MAX_DURATION * 60 {
        warn!("The audio message is above {MAX_DURATION} minutes!");
        return Err(format!("Duration is above {MAX_DURATION} minutes"));
    }

    Ok(())
}

async fn set_reaction(bot: &Bot, message: &Message, emoji: &str) {
    if let Err(err) = bot
        .set_message_reaction(message.chat.id, message.id)
        .reaction([ReactionType::Emoji {
            emoji: emoji.to_string(),
        }])
        .await
    {
        error!("Failed to set reaction: {err:?}");
    }
}

async fn consume_rate_limit(
    bot: &Bot,
    reply_context: &Message,
    db: &Arc<Database>,
    user_id: UserId,
) -> bool {
    match db
        .check_and_record_usage(user_id, RATE_LIMIT_PER_MINUTE, RATE_LIMIT_PER_HOUR)
        .await
    {
        Ok(true) => true,
        Ok(false) => {
            warn!("Rate limit exceeded for user {}", user_id);
            set_reaction(bot, reply_context, USER_RATE_LIMIT_REACTION).await;
            false
        }
        Err(e) => {
            error!("Failed to check rate limit: {e}");
            true
        }
    }
}

/// Reserves one llama.cpp request. Reacts with 🙊 when the user has used up
/// their share and with 😴 when the whole bot is at its limit.
async fn consume_llm_quota(
    bot: &Bot,
    reply_context: &Message,
    db: &Arc<Database>,
    user_id: UserId,
) -> bool {
    match db.check_and_record_llm_usage(user_id, &LLM_LIMITS).await {
        Ok(LlmUsage::Allowed) => true,
        Ok(LlmUsage::UserLimited) => {
            set_reaction(bot, reply_context, USER_RATE_LIMIT_REACTION).await;
            false
        }
        Ok(LlmUsage::GlobalLimited) => {
            set_reaction(bot, reply_context, BUSY_REACTION).await;
            false
        }
        Err(e) => {
            error!("Failed to check LLM limit: {e}");
            true
        }
    }
}

/// Returns the transcript of the audio from the cache, or from Phonon for
/// `TaskType::Phonon` and whisper.cpp otherwise. An empty string means the
/// audio has no speech. Returns `None` after it has already told the user why
/// there is no transcript.
async fn get_transcript(
    bot: &Bot,
    db: &Arc<Database>,
    audio_info: &AudioFileInfo,
    reply_context: &Message,
    transcript_type: TaskType,
) -> Option<String> {
    let cache_key = audio_info.unique_id.to_string();
    match db.get_cached(&cache_key, &transcript_type).await {
        Ok(Some(transcript)) => {
            info!(
                "Transcript found in cache for unique_file_id: {}",
                audio_info.unique_id
            );
            return Some(transcript);
        }
        Err(e) => error!("Failed to read transcript cache: {e}"),
        Ok(None) => {}
    }

    if let Err(error_msg) = validate_audio_limits(audio_info) {
        safe_send(bot, reply_context, Some(&error_msg), None, None).await;
        return None;
    }

    let (audio_bytes, mime, duration) = match download_audio(bot, audio_info).await {
        Ok(res) => res,
        Err(e) => {
            error!("Failed to download audio: {e:?}");
            safe_send(bot, reply_context, Some(&format!("Error: {e}")), None, None).await;
            return None;
        }
    };

    info!("Transcribing audio! Duration: {duration} | Mime: {mime:?}");
    let now = std::time::Instant::now();
    let result = match transcript_type {
        TaskType::Phonon => phonon::transcribe(audio_bytes).await,
        _ => transcribe::transcribe(audio_bytes, mime).await,
    };
    let transcript = match result {
        Ok(transcript) => transcript.unwrap_or_default().trim().to_string(),
        Err(e) => {
            warn!("Failed to transcribe audio: {e}");
            set_reaction(bot, reply_context, FAILED_REACTION).await;
            return None;
        }
    };
    info!("Transcribed audio in {}ms", now.elapsed().as_millis());

    if let Err(e) = db
        .set_cached(&cache_key, &transcript_type, &transcript)
        .await
    {
        error!("Failed to cache transcript: {e}");
    }

    Some(transcript)
}

async fn handle_audio_message(
    audio_source_message: &Message,
    reply_context: &Message,
    bot: &Bot,
    db: &Arc<Database>,
    task_type: TaskType,
    user_id: UserId,
) {
    let audio_info = match setup_audio_processing(audio_source_message).await {
        Some(info) => info,
        None => return,
    };

    if !consume_rate_limit(bot, reply_context, db, user_id).await {
        return;
    }

    let cache_key = audio_info.unique_id.to_string();
    if matches!(task_type, TaskType::Translate) {
        match db.get_cached(&cache_key, &task_type).await {
            Ok(Some(translation)) => {
                info!(
                    "Translation found in cache for unique_file_id: {}",
                    audio_info.unique_id
                );
                send_transcript(bot, reply_context, &translation, task_type).await;
                return;
            }
            Err(e) => error!("Failed to read translation cache: {e}"),
            Ok(None) => {}
        }

        if audio_info.duration > MAX_TRANSLATE_DURATION * 60 {
            let message = format!(
                "Translation is limited to recordings up to {MAX_TRANSLATE_DURATION} minutes."
            );
            safe_send(bot, reply_context, Some(&message), None, None).await;
            return;
        }
    }

    let typing_guard = start_typing_indicator(bot.clone(), reply_context.chat.id);

    // Only /phonon uses Phonon; translations start from the whisper.cpp transcript.
    let transcript_type = match task_type {
        TaskType::Phonon => TaskType::Phonon,
        _ => TaskType::Transcribe,
    };
    let Some(transcript) =
        get_transcript(bot, db, &audio_info, reply_context, transcript_type).await
    else {
        return;
    };

    let output = match task_type {
        TaskType::Translate if !transcript.is_empty() => {
            if !consume_llm_quota(bot, reply_context, db, user_id).await {
                return;
            }
            match llm::translate(&transcript).await {
                Ok(translation) => {
                    if let Err(e) = db.set_cached(&cache_key, &task_type, &translation).await {
                        error!("Failed to cache translation: {e}");
                    }
                    translation
                }
                Err(e) => {
                    warn!("Failed to translate transcript: {e}");
                    set_reaction(bot, reply_context, FAILED_REACTION).await;
                    return;
                }
            }
        }
        _ if transcript.is_empty() => "<no text>".to_string(),
        _ => transcript,
    };

    drop(typing_guard);

    send_transcript(bot, reply_context, &output, task_type).await;
}

async fn handle_summarization(
    audio_source_message: &Message,
    reply_context: &Message,
    method: SummarizeMethod,
    task_type: TaskType,
    bot: &Bot,
    db: &Arc<Database>,
    user_id: UserId,
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

    if !consume_rate_limit(bot, reply_context, db, user_id).await {
        return;
    }

    let cache_key = audio_info.unique_id.to_string();
    match db.get_cached(&cache_key, &task_type).await {
        Ok(Some(cached_summary)) => {
            info!(
                "Summary found in cache for unique_file_id: {}",
                audio_info.unique_id
            );
            send_summary(bot, reply_context, &cached_summary, task_type).await;
            return;
        }
        Err(e) => error!("Failed to read summary cache: {e}"),
        Ok(None) => {}
    }

    let typing_guard = start_typing_indicator(bot.clone(), reply_context.chat.id);

    let Some(transcript) =
        get_transcript(bot, db, &audio_info, reply_context, TaskType::Transcribe).await
    else {
        return;
    };

    if transcript.is_empty() {
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

    if !consume_llm_quota(bot, reply_context, db, user_id).await {
        return;
    }

    // The model summarizes straight from the original language into English.
    let summary = match llm::summarize(&transcript, method).await {
        Ok(summary) => summary,
        Err(e) => {
            warn!("Failed to summarize transcript: {e}");
            set_reaction(bot, reply_context, FAILED_REACTION).await;
            return;
        }
    };

    drop(typing_guard);

    send_summary(bot, reply_context, &summary, task_type).await;

    if let Err(e) = db.set_cached(&cache_key, &task_type, &summary).await {
        error!("Failed to cache summary: {e}");
    }
}

async fn send_summary(bot: &Bot, reply_context: &Message, summary: &str, task_type: TaskType) {
    let formatted_summary = format!("<i>{}</i>", html::escape(summary));
    safe_send(
        bot,
        reply_context,
        Some(&formatted_summary),
        Some(ParseMode::Html),
        Some(task_type),
    )
    .await;
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
