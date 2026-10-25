use crate::types::TaskType;
use crate::utils::post_json;
use log::error;
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;

pub const LLM_MODEL: &str = "Nemotron 3.5 Bolt E4B";
const DEFAULT_LLM_URL: &str = "http://host.docker.internal:8081/v1/chat/completions";
// Translating a 10 minute recording takes about two and a half minutes on the
// shared laptop GPU.
const LLM_TIMEOUT: Duration = Duration::from_secs(300);

const TRANSLATE_PROMPT: &str = "You translate transcriptions of voice messages into English. Output only the English translation, keeping the speaker's meaning, tone and first-person voice. Do not add notes, quotes or explanations. If the text is already English, output it unchanged.";
const SUMMARY_PROMPT: &str = "You are an AI that explains transcriptions of voice messages. Don't speak as the user, instead describe what the user is saying. Always provide the summary in English, ensuring it is concise yet comprehensive. If the content is unclear, nonsensical, or you're unsure about the message's meaning, respond **only** with three question marks (`???`). Do not include any additional text, explanations, or formatting—output **strictly** the summary or `???`.";
const CAVEMAN_PROMPT: &str = "You are an AI that explains transcriptions of voice messages like a caveman. Don't speak as the user, instead describe what the user is saying in caveman language, always in English. Use all caps, no verbs. If the content is unclear, nonsensical, or you're unsure about the message's meaning, respond **only** with three question marks (`???`). Do not include any additional text, explanations, or formatting—output **strictly** the summary or `???`.";

/// What llama.cpp is asked to do with a transcript.
pub struct Prompt {
    system: &'static str,
    temperature: f32,
    max_tokens: u32,
}

/// The prompt for `task`, or `None` when the transcript itself is the result.
pub fn prompt(task: TaskType) -> Option<Prompt> {
    let (system, temperature, max_tokens) = match task {
        TaskType::Transcribe | TaskType::Phonon => return None,
        TaskType::Translate => (TRANSLATE_PROMPT, 0.2, 8192),
        TaskType::Summarize => (SUMMARY_PROMPT, 0.4, 512),
        TaskType::Caveman => (CAVEMAN_PROMPT, 0.7, 512),
    };
    Some(Prompt {
        system,
        temperature,
        max_tokens,
    })
}

/// Runs `prompt` on the transcript with the local llama.cpp server.
pub async fn chat(prompt: Prompt, transcript: &str) -> Result<String, String> {
    let url = env::var("LLM_URL").unwrap_or_else(|_| DEFAULT_LLM_URL.to_string());
    let request = ChatRequest {
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: prompt.system.to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: transcript.to_string(),
            },
        ],
        temperature: prompt.temperature,
        top_p: 0.95,
        max_tokens: prompt.max_tokens,
        chat_template_kwargs: ChatTemplateKwargs {
            enable_thinking: false,
        },
    };

    let res: ChatResponse = post_json(
        "llama.cpp",
        reqwest::Client::new()
            .post(url)
            .timeout(LLM_TIMEOUT)
            .json(&request),
    )
    .await?;

    let content = res
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content.trim().to_string())
        .unwrap_or_default();

    if content.is_empty() {
        error!("llama.cpp returned an empty answer");
        return Err("llama.cpp returned an empty answer".to_string());
    }

    Ok(content)
}

#[derive(Serialize)]
struct ChatRequest {
    messages: Vec<ChatMessage>,
    temperature: f32,
    top_p: f32,
    max_tokens: u32,
    chat_template_kwargs: ChatTemplateKwargs,
}

#[derive(Serialize)]
struct ChatTemplateKwargs {
    enable_thinking: bool,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}
