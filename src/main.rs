use chrono::Local;
use database::{Cached, Database, LlmLimits, LlmUsage};
use dotenvy::dotenv;
use log::{debug, info, warn};
use std::env;
use std::sync::Arc;
use std::time::Instant;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{FileMeta, Me, ParseMode};
use teloxide::utils::command::BotCommands;
use types::{BotCommand, TaskType, audio_file};
use utils::{reply, send_result, set_reaction, start_typing_indicator};

mod database;
mod llm;
mod media;
mod phonon;
mod transcribe;
mod types;
mod utils;

const RATE_LIMIT_PER_MINUTE: usize = 25;
const RATE_LIMIT_PER_HOUR: usize = 150;
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

const START_TEXT: &str = "Welcome! Send a voice message or video note to transcribe it. You can also use /help to see all available commands.";
const DONATE_TEXT: &str = "<b>Support duck_transcriber_bot</b>\n\
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

#[tokio::main]
async fn main() {
    dotenv().ok();
    setup_logging().expect("Failed to setup logging");

    info!("Starting duck_transcriber bot");

    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set!"));
    let db = Arc::new(Database::default());

    let res = bot.set_my_commands(BotCommand::bot_commands()).await;
    if let Err(e) = res {
        warn!("Failed to set commands: {e:?}");
    }

    let db_cleanup = db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            db_cleanup.cleanup_expired();
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

/// `me` is the bot's own account, which the dispatcher fetches once at startup.
async fn handle_message(
    bot: Bot,
    me: Me,
    message: Message,
    db: Arc<Database>,
) -> ResponseResult<()> {
    let Some(user_id) = message.from.as_ref().map(|user| user.id) else {
        debug!("Received message without user info");
        return Ok(());
    };

    // A command can also be the caption of the audio it is about.
    let command = message
        .text()
        .or(message.caption())
        .and_then(|text| BotCommand::parse(text, me.username()).ok());
    if let Some(command) = command {
        handle_command(&bot, &message, command, &db, user_id).await;
        return Ok(());
    }

    // Voice messages and video notes are transcribed everywhere, videos and
    // audio files only in DMs.
    let auto =
        message.voice().is_some() || message.video_note().is_some() || message.chat.is_private();
    if auto && let Some(file) = audio_file(&message) {
        handle_audio(&bot, &db, file, &message, TaskType::Transcribe, user_id).await;
    }

    Ok(())
}

async fn handle_command(
    bot: &Bot,
    message: &Message,
    command: BotCommand,
    db: &Database,
    user_id: UserId,
) {
    let html = Some(ParseMode::Html);
    let (task, usage) = match command {
        BotCommand::Help => {
            return reply(bot, message, &BotCommand::descriptions().to_string(), None).await;
        }
        BotCommand::Start => return reply(bot, message, START_TEXT, None).await,
        BotCommand::Privacy => return reply(bot, message, &privacy_text(), html).await,
        BotCommand::Limits => return reply(bot, message, &limits_text(), html).await,
        BotCommand::Donate => return reply(bot, message, DONATE_TEXT, html).await,
        BotCommand::Transcribe => (TaskType::Transcribe, "transcribe it"),
        BotCommand::Phonon => (TaskType::Phonon, "transcribe it with Phonon-2.5"),
        BotCommand::Translate => (TaskType::Translate, "translate it"),
        BotCommand::Summarize => (TaskType::Summarize, "summarize it"),
        BotCommand::Caveman => (TaskType::Caveman, "summarize it like a caveman"),
    };

    // The audio is in the command's own message, with the command as its
    // caption, or in the message the command replies to.
    match std::iter::once(message)
        .chain(message.reply_to_message())
        .find_map(audio_file)
    {
        Some(file) => handle_audio(bot, db, file, message, task, user_id).await,
        None => {
            let help = format!("Reply to an audio message or video note to {usage}.");
            reply(bot, message, &help, None).await;
        }
    }
}

fn privacy_text() -> String {
    format!(
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
    )
}

fn limits_text() -> String {
    format!(
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
    )
}

/// Why an audio request ended without a result, and how the user hears about it.
enum Stop {
    /// Replies with this text: limits and download errors the user can act on.
    Reply(String),
    /// Reacts instead, so failures and limits stay quiet in groups.
    React(&'static str),
}

/// Runs `task` on `file` and answers `request`. Every audio command and every
/// automatic transcription goes through here.
async fn handle_audio(
    bot: &Bot,
    db: &Database,
    file: &FileMeta,
    request: &Message,
    task: TaskType,
    user_id: UserId,
) {
    info!("Received audio {} for {task}", file.unique_id);
    // Checked before the rate limit so a chat where the bot can't post doesn't
    // use up the sender's quota.
    let Some(typing) = start_typing_indicator(bot.clone(), request.chat.id).await else {
        return;
    };
    let result = run_task(bot, db, file, task, user_id).await;
    drop(typing);

    match result {
        Ok(text) => send_result(bot, request, &text, task).await,
        Err(Stop::Reply(text)) => reply(bot, request, &text, None).await,
        Err(Stop::React(emoji)) => set_reaction(bot, request, emoji).await,
    }
}

/// The result of `task`, from the cache when it can. Cached results cost no LLM
/// quota and skip the length limits, which they passed when they were made.
async fn run_task(
    bot: &Bot,
    db: &Database,
    file: &FileMeta,
    task: TaskType,
    user_id: UserId,
) -> Result<String, Stop> {
    if !db.check_and_record_usage(user_id, RATE_LIMIT_PER_MINUTE, RATE_LIMIT_PER_HOUR) {
        return Err(Stop::React(USER_RATE_LIMIT_REACTION));
    }

    let prompt = llm::prompt(task);
    // Transcripts are cached by get_transcript; llama.cpp results have their own
    // slots.
    if prompt.is_some()
        && let Some(cached) = db.get_cached(&file.unique_id, task)
    {
        info!(
            "Found the {task} result for {} in the cache",
            file.unique_id
        );
        return Ok(cached.text);
    }

    let transcript = get_transcript(bot, db, file, task).await?;
    // Recordings without speech never reach llama.cpp, so they use no LLM quota.
    if transcript.text.is_empty() {
        return match task {
            TaskType::Summarize | TaskType::Caveman => {
                Err(Stop::Reply("No text found in audio".to_string()))
            }
            _ => Ok("<no text>".to_string()),
        };
    }
    let Some(prompt) = prompt else {
        return Ok(transcript.text);
    };

    match db.check_and_record_llm_usage(user_id, &LLM_LIMITS) {
        LlmUsage::Allowed => {}
        LlmUsage::UserLimited => return Err(Stop::React(USER_RATE_LIMIT_REACTION)),
        LlmUsage::GlobalLimited => return Err(Stop::React(BUSY_REACTION)),
    }

    info!("Asking {} to {task}", llm::LLM_MODEL);
    // The model translates and summarizes straight from the original language.
    let text = llm::chat(prompt, &transcript.text).await.map_err(|e| {
        warn!("Failed to {task} the transcript: {e}");
        Stop::React(FAILED_REACTION)
    })?;

    let result = Cached {
        text,
        seconds: transcript.seconds,
    };
    db.set_cached(&file.unique_id, task, result.clone());
    Ok(result.text)
}

/// The transcript `task` starts from, with the decoded length of its
/// recording: from the cache, or downloaded, decoded and transcribed now.
/// Recordings too long for `task` are refused before anything is transcribed.
async fn get_transcript(
    bot: &Bot,
    db: &Database,
    file: &FileMeta,
    task: TaskType,
) -> Result<Cached, Stop> {
    // Only /phonon uses Phonon; translations and summaries start from the
    // whisper.cpp transcript, so they reuse what /transcribe cached.
    let source = match task {
        TaskType::Phonon => TaskType::Phonon,
        _ => TaskType::Transcribe,
    };
    if let Some(cached) = db.get_cached(&file.unique_id, source) {
        info!("Found the transcript of {} in the cache", file.unique_id);
        // Its length was measured when it was transcribed, so /translate's
        // limit holds without downloading the recording again.
        check_length(task, cached.seconds).map_err(Stop::Reply)?;
        return Ok(cached);
    }

    let audio = download(bot, file).await?;
    let wav = media::to_wav(audio, max_seconds(task))
        .await
        .ok_or(Stop::React(FAILED_REACTION))?;
    check_length(task, wav.seconds).map_err(Stop::Reply)?;

    let engine = match source {
        TaskType::Phonon => phonon::PHONON_MODEL,
        _ => transcribe::WHISPER_MODEL,
    };
    info!("Transcribing {}s of audio with {engine}", wav.seconds);
    let started = Instant::now();
    let text = match source {
        TaskType::Phonon => phonon::transcribe(wav.bytes).await,
        _ => transcribe::transcribe(wav.bytes).await,
    }
    .map_err(|e| {
        warn!("Failed to transcribe audio: {e}");
        Stop::React(FAILED_REACTION)
    })?;
    info!("Transcribed audio in {}ms", started.elapsed().as_millis());

    let transcript = Cached {
        text: text.trim().to_string(),
        seconds: wav.seconds,
    };
    db.set_cached(&file.unique_id, source, transcript.clone());
    Ok(transcript)
}

/// Downloads the file. Its size, unlike its length, comes from Telegram's
/// servers, and the Bot API can't download bigger files anyway; the check just
/// tells the user why.
async fn download(bot: &Bot, file: &FileMeta) -> Result<Vec<u8>, Stop> {
    if file.size > MAX_FILE_SIZE * 1024 * 1024 {
        warn!("File is larger than {MAX_FILE_SIZE}MB");
        return Err(Stop::Reply(format!(
            "File can't be larger than {MAX_FILE_SIZE}MB (is {:.2}MB)",
            file.size as f64 / 1024.0 / 1024.0
        )));
    }

    let failed = |e: String| {
        warn!("Failed to download audio: {e}");
        Stop::Reply(format!("Error: {e}"))
    };
    let path = bot
        .get_file(file.id.clone())
        .await
        .map_err(|e| failed(format!("Failed to get file: {e}")))?
        .path;
    let mut audio = Vec::new();
    bot.download_file(&path, &mut audio)
        .await
        .map_err(|e| failed(format!("Failed to download file: {e}")))?;
    Ok(audio)
}

/// The longest recording `task` accepts, in seconds.
fn max_seconds(task: TaskType) -> u32 {
    60 * match task {
        TaskType::Translate => MAX_TRANSLATE_DURATION,
        _ => MAX_DURATION,
    }
}

/// Refuses recordings longer than `task` accepts. `seconds` is always the
/// decoded length, never the one Telegram reports.
fn check_length(task: TaskType, seconds: u32) -> Result<(), String> {
    if seconds <= max_seconds(task) {
        return Ok(());
    }
    warn!("Refusing {seconds}s of audio for {task}");
    Err(match task {
        TaskType::Translate => {
            format!("Translation is limited to recordings up to {MAX_TRANSLATE_DURATION} minutes.")
        }
        _ => format!("Duration is above {MAX_DURATION} minutes"),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_has_its_own_length_limit() {
        let translate_limit = "Translation is limited to recordings up to 10 minutes.";
        assert!(check_length(TaskType::Translate, 600).is_ok());
        assert_eq!(
            check_length(TaskType::Translate, 601).unwrap_err(),
            translate_limit
        );
        assert_eq!(
            check_length(TaskType::Translate, 3600).unwrap_err(),
            translate_limit
        );
        assert!(check_length(TaskType::Summarize, 1800).is_ok());
        assert_eq!(
            check_length(TaskType::Phonon, 1801).unwrap_err(),
            "Duration is above 30 minutes"
        );
    }
}
