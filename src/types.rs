use mime::Mime;
use serde::{Deserialize, Serialize};
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
    #[command(description = "transcribe & translate the replied audio file in English.", aliases = ["english", "en"])]
    Translate,
    #[command(description = "summarize the replied audio message")]
    Summarize,
    #[command(description = "summarize the replied audio message like a caveman")]
    Caveman,
    #[command(description = "show privacy policy")]
    Privacy,
}

pub enum AudioAction {
    Transcribe(TaskType),
    Summarize(SummarizeMethod),
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

#[derive(strum::Display)]
pub enum TaskType {
    #[strum(to_string = "transcribe")]
    Transcribe,
    #[strum(to_string = "translate")]
    Translate,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GroqWhisperResponse {
    pub task: String,
    pub language: String,
    pub duration: f64,
    pub text: String,
    pub segments: Vec<GroqWhisperSegment>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GroqWhisperSegment {
    pub id: u32,
    pub seek: u32,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub tokens: Vec<u32>,
    pub temperature: f64,
    pub avg_logprob: f64,
    pub compression_ratio: f64,
    pub no_speech_prob: f64,
}

pub enum SummarizeMethod {
    Default,
    Caveman,
}

#[derive(Debug, Serialize)]
pub struct GroqChatRequest {
    pub model: String,
    pub messages: Vec<GroqChatMessage>,
    pub temperature: f32,
    pub max_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GroqChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct GroqChatResponse {
    pub choices: Vec<GroqChatChoice>,
}

#[derive(Debug, Deserialize)]
pub struct GroqChatChoice {
    pub message: GroqChatMessage,
}

pub struct DBItem {
    pub text: String,
    pub unique_file_id: String, // Using String for compatibility with DynamoDB
    pub task_type: String,
    pub expires_at: i64, // Unix timestamp for TTL
}

pub enum ItemReturnInfo {
    Text(String),
    Exists, // Item already exists, but for other task type.
    None,
}
