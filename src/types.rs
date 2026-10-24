use mime::Mime;
use std::str::FromStr;
use teloxide::types::{FileId, FileUniqueId, Message};
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

pub enum AudioAction {
    Transcribe,
    Phonon,
    Translate,
    Summarize(SummarizeMethod),
}

impl AudioAction {
    pub fn task_type(&self) -> TaskType {
        match self {
            AudioAction::Transcribe => TaskType::Transcribe,
            AudioAction::Phonon => TaskType::Phonon,
            AudioAction::Translate => TaskType::Translate,
            AudioAction::Summarize(SummarizeMethod::Default) => TaskType::Summarize,
            AudioAction::Summarize(SummarizeMethod::Caveman) => TaskType::Caveman,
        }
    }
}

#[derive(Debug)]
pub struct AudioFileInfo {
    pub file_id: FileId,
    pub unique_id: FileUniqueId,
    pub duration: u32,
    pub size: u32,
    pub kind: AudioSourceKind,
    pub mime: Option<Mime>,
}

impl AudioFileInfo {
    pub fn from_message(message: &Message) -> Option<Self> {
        if let Some(voice) = message.voice() {
            return Some(Self {
                file_id: voice.file.id.clone(),
                unique_id: voice.file.unique_id.clone(),
                duration: voice.duration.seconds(),
                size: voice.file.size,
                kind: AudioSourceKind::Voice,
                mime: voice.mime_type.clone(),
            });
        }
        if let Some(video_note) = message.video_note() {
            return Some(Self {
                file_id: video_note.file.id.clone(),
                unique_id: video_note.file.unique_id.clone(),
                duration: video_note.duration.seconds(),
                size: video_note.file.size,
                kind: AudioSourceKind::VideoNote,
                mime: Some(Mime::from_str("video/mp4").unwrap()),
            });
        }
        if let Some(video) = message.video() {
            return Some(Self {
                file_id: video.file.id.clone(),
                unique_id: video.file.unique_id.clone(),
                duration: video.duration.seconds(),
                size: video.file.size,
                kind: AudioSourceKind::Video,
                mime: video.mime_type.clone(),
            });
        }
        if let Some(audio) = message.audio() {
            return Some(Self {
                file_id: audio.file.id.clone(),
                unique_id: audio.file.unique_id.clone(),
                duration: audio.duration.seconds(),
                size: audio.file.size,
                kind: AudioSourceKind::Audio,
                mime: audio.mime_type.clone(),
            });
        }
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AudioSourceKind {
    Voice,
    VideoNote,
    Video,
    Audio,
}

#[derive(strum::Display, Clone, Copy)]
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

#[derive(Clone, Copy)]
pub enum SummarizeMethod {
    Default,
    Caveman,
}

#[derive(Debug)]
pub enum TranscriptionError {
    Network(String),
    Parse(String),
    Api(String),
}

impl std::fmt::Display for TranscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscriptionError::Network(msg) => write!(f, "Network error: {}", msg),
            TranscriptionError::Parse(msg) => write!(f, "Parse error: {}", msg),
            TranscriptionError::Api(msg) => write!(f, "API error: {}", msg),
        }
    }
}
