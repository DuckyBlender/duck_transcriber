use teloxide::types::{FileMeta, Message};
use teloxide::utils::command::BotCommands;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum BotCommand {
    #[command(description = "display this text")]
    Help,
    #[command(description = "welcome message")]
    Start,
    #[command(description = "transcribe the replied audio")]
    Transcribe,
    #[command(description = "transcribe the replied audio with Phonon-2.5 instead of Whisper")]
    Phonon,
    #[command(description = "transcribe & translate the replied audio file in English.", aliases = ["english", "en"])]
    Translate,
    #[command(description = "summarize the replied audio message")]
    Summarize,
    #[command(description = "summarize the replied audio message like a caveman")]
    Caveman,
    #[command(description = "show privacy policy")]
    Privacy,
    #[command(description = "show current rate limit", aliases = ["ratelimits", "ratelimit"])]
    Limits,
    #[command(description = "show donation addresses")]
    Donate,
}

/// What the bot does with a recording. Each task has its own cache slot.
#[derive(strum::Display, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskType {
    #[strum(to_string = "transcribe")]
    Transcribe,
    #[strum(to_string = "phonon")]
    Phonon,
    #[strum(to_string = "translate")]
    Translate,
    #[strum(to_string = "summarize")]
    Summarize,
    #[strum(to_string = "caveman")]
    Caveman,
}

/// The file of the voice message, video note, video or audio file in `message`.
/// The duration next to it is left out on purpose: the sender's app sets it, so
/// it can be 0 or anything else.
pub fn audio_file(message: &Message) -> Option<&FileMeta> {
    message
        .voice()
        .map(|voice| &voice.file)
        .or_else(|| message.video_note().map(|note| &note.file))
        .or_else(|| message.video().map(|video| &video.file))
        .or_else(|| message.audio().map(|audio| &audio.file))
}
