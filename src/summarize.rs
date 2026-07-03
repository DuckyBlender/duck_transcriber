use crate::BASE_URL;
use crate::types::{
    GroqChatMessage, GroqChatRequest, GroqChatResponse, SummarizeMethod, TranscriptionError,
};
use crate::utils;
use log::{error, info, warn};
use reqwest::header::AUTHORIZATION;
use reqwest::header::HeaderMap;

pub const SUMMARIZATION_MODELS: [&str; 3] = [
    "qwen/qwen3.6-27b",
    "openai/gpt-oss-120b",
    "openai/gpt-oss-20b",
];

pub async fn summarize(text: &str, method: SummarizeMethod) -> Result<String, TranscriptionError> {
    let api_keys = utils::get_api_keys();

    if api_keys.is_empty() {
        error!("No API keys configured");
        return Err(TranscriptionError::ApiError(
            "API key not configured".to_string(),
        ));
    }

    let mut last_error = None;
    for (key_index, api_key) in api_keys.iter().enumerate() {
        for model in SUMMARIZATION_MODELS {
            info!(
                "Attempting summarization with model {} (key {} of {})",
                model,
                key_index + 1,
                api_keys.len()
            );

            match summarize_with_key(text, method, api_key, model).await {
                Ok(result) => return Ok(result),
                Err(TranscriptionError::RateLimitReached) => {
                    warn!(
                        "Rate limit reached with model {}, key {}",
                        model,
                        key_index + 1
                    );
                    last_error = Some(TranscriptionError::RateLimitReached);
                }
                Err(e) => {
                    error!(
                        "Error with key {} and model {}: {}",
                        key_index + 1,
                        model,
                        e
                    );
                    last_error = Some(e);
                    break;
                }
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| TranscriptionError::ApiError("All API keys failed".to_string())))
}

async fn summarize_with_key(
    text: &str,
    method: SummarizeMethod,
    api_key: &str,
    model: &str,
) -> Result<String, TranscriptionError> {
    let mut headers = HeaderMap::new();

    let auth_value = format!("Bearer {}", api_key).parse().map_err(|e| {
        error!("Failed to parse authorization header: {e}");
        TranscriptionError::ParseError("Invalid API key format".to_string())
    })?;

    headers.insert(AUTHORIZATION, auth_value);

    let system_prompt = match method {
        SummarizeMethod::Default => {
            "You are an AI that explains transcriptions of voice messages. Don't speak as the user, instead describe what the user is saying. Always provide the summary in English, ensuring it is concise yet comprehensive. If the content is unclear, nonsensical, or you're unsure about the message's meaning, respond **only** with three question marks (`???`). Do not include any additional text, explanations, or formatting—output **strictly** the summary or `???`."
        }
        SummarizeMethod::Caveman => {
            "You are an AI that explains transcriptions of voice messages like a caveman. Don't speak as the user, instead describe what the user is saying in caveman language. Use all caps, no verbs. If the content is unclear, nonsensical, or you're unsure about the message's meaning, respond **only** with three question marks (`???`). Do not include any additional text, explanations, or formatting—output **strictly** the summary or `???`."
        }
    };

    let temperature = match method {
        SummarizeMethod::Default => 0.4,
        SummarizeMethod::Caveman => 0.7,
    };

    let request = GroqChatRequest {
        model: model.to_string(),
        messages: vec![
            GroqChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            GroqChatMessage {
                role: "user".to_string(),
                content: text.to_string(),
            },
        ],
        temperature,
        max_tokens: 512,
    };

    let client = reqwest::Client::new();
    let res = client
        .post(format!("{BASE_URL}/chat/completions"))
        .headers(headers)
        .json(&request)
        .send()
        .await
        .map_err(|err| {
            error!("Failed to send request to Groq: {err}");
            TranscriptionError::NetworkError(format!("Failed to send request: {err}"))
        })?;

    if !res.status().is_success() {
        let json = res.json::<serde_json::Value>().await.map_err(|err| {
            error!("Failed to parse Groq error response: {err}");
            TranscriptionError::ParseError("Failed to parse API error response".to_string())
        })?;

        // Check for rate limit
        if json["error"]["code"] == "rate_limit_exceeded" {
            warn!("Rate limit reached for chat API");
            return Err(TranscriptionError::RateLimitReached);
        }

        let error_msg = json["error"]["message"].as_str().unwrap_or("unknown error");
        error!("Groq returned an error: {error_msg}");
        return Err(TranscriptionError::ApiError(format!(
            "Groq error: {}",
            error_msg
        )));
    }

    let response = res.json::<GroqChatResponse>().await.map_err(|err| {
        error!("Failed to parse Groq response: {err}");
        TranscriptionError::ParseError("Failed to parse API response".to_string())
    })?;

    let choice = response.choices.first().ok_or_else(|| {
        error!("Groq returned a chat response with no choices");
        TranscriptionError::ParseError("Groq response did not include a summary".to_string())
    })?;

    Ok(choice.message.content.trim().to_string())
}
