use crate::transcribe::truncate_for_log;
use crate::types::{SummarizeMethod, TranscriptionError};
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;

pub const LLM_MODEL: &str = "Nemotron 3.5 Bolt E4B";
const DEFAULT_LLM_URL: &str = "http://host.docker.internal:8081/v1/chat/completions";
// Long translations of 30 minute recordings take a while on the shared laptop GPU.
const LLM_TIMEOUT: Duration = Duration::from_secs(300);

const TRANSLATE_PROMPT: &str = "You translate transcriptions of voice messages into English. Output only the English translation, keeping the speaker's meaning, tone and first-person voice. Do not add notes, quotes or explanations. If the text is already English, output it unchanged.";
const SUMMARY_PROMPT: &str = "You are an AI that explains transcriptions of voice messages. Don't speak as the user, instead describe what the user is saying. Always provide the summary in English, ensuring it is concise yet comprehensive. If the content is unclear, nonsensical, or you're unsure about the message's meaning, respond **only** with three question marks (`???`). Do not include any additional text, explanations, or formatting—output **strictly** the summary or `???`.";
const CAVEMAN_PROMPT: &str = "You are an AI that explains transcriptions of voice messages like a caveman. Don't speak as the user, instead describe what the user is saying in caveman language, always in English. Use all caps, no verbs. If the content is unclear, nonsensical, or you're unsure about the message's meaning, respond **only** with three question marks (`???`). Do not include any additional text, explanations, or formatting—output **strictly** the summary or `???`.";

pub async fn translate(text: &str) -> Result<String, TranscriptionError> {
    info!("Translating with {LLM_MODEL}");
    chat(TRANSLATE_PROMPT, text, 0.2, 8192).await
}

pub async fn summarize(text: &str, method: SummarizeMethod) -> Result<String, TranscriptionError> {
    info!("Summarizing with {LLM_MODEL}");
    match method {
        SummarizeMethod::Default => chat(SUMMARY_PROMPT, text, 0.4, 512).await,
        SummarizeMethod::Caveman => chat(CAVEMAN_PROMPT, text, 0.7, 512).await,
    }
}

async fn chat(
    system: &str,
    user: &str,
    temperature: f32,
    max_tokens: u32,
) -> Result<String, TranscriptionError> {
    let url = env::var("LLM_URL").unwrap_or_else(|_| DEFAULT_LLM_URL.to_string());
    let request = ChatRequest {
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: system.to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: user.to_string(),
            },
        ],
        temperature,
        top_p: 0.95,
        max_tokens,
        chat_template_kwargs: ChatTemplateKwargs {
            enable_thinking: false,
        },
    };

    let res = reqwest::Client::new()
        .post(&url)
        .timeout(LLM_TIMEOUT)
        .json(&request)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to llama.cpp at {url}: {err}");
            TranscriptionError::Network(format!("Failed to send request to llama.cpp: {err}"))
        })?;

    let status = res.status();
    let body = res.text().await.map_err(|err| {
        error!("Failed to read llama.cpp response body ({status}): {err}");
        TranscriptionError::Network(format!("Failed to read llama.cpp response body: {err}"))
    })?;

    if !status.is_success() {
        error!("llama.cpp returned {status}: {}", truncate_for_log(&body));
        return Err(TranscriptionError::Api(format!(
            "llama.cpp error: {status}"
        )));
    }

    let response: ChatResponse = serde_json::from_str(&body).map_err(|err| {
        error!(
            "Failed to parse llama.cpp response: {err}; body: {}",
            truncate_for_log(&body)
        );
        TranscriptionError::Parse("Failed to parse llama.cpp response".to_string())
    })?;

    let content = response
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content.trim().to_string())
        .unwrap_or_default();

    if content.is_empty() {
        error!("llama.cpp returned an empty answer");
        return Err(TranscriptionError::Api(
            "llama.cpp returned an empty answer".to_string(),
        ));
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
