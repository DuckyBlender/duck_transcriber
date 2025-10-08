use crate::BASE_URL;
use crate::types::{
    GroqChatMessage, GroqChatRequest, GroqChatResponse, SummarizeMethod, TranscriptionError,
};
use crate::utils;
use log::{error, info, warn};
use reqwest::header::AUTHORIZATION;
use reqwest::header::HeaderMap;

pub async fn summarize(text: &str, method: SummarizeMethod) -> Result<String, TranscriptionError> {
    let api_keys = utils::get_api_keys();

    if api_keys.is_empty() {
        error!("No API keys configured");
        return Err(TranscriptionError::ApiError(
            "API key not configured".to_string(),
        ));
    }

    // Try each API key until one succeeds
    let mut last_error = None;
    for (attempt, api_key) in api_keys.iter().enumerate() {
        info!(
            "Attempting summarization with API key {} of {}",
            attempt + 1,
            api_keys.len()
        );

        match summarize_with_key(text, method, api_key).await {
            Ok(result) => return Ok(result),
            Err(TranscriptionError::RateLimitReached) => {
                warn!(
                    "Rate limit reached with key {}, trying next key",
                    attempt + 1
                );
                last_error = Some(TranscriptionError::RateLimitReached);
                continue;
            }
            Err(e) => {
                error!("Error with key {}: {}", attempt + 1, e);
                last_error = Some(e);
                break;
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
        model: "moonshotai/kimi-k2-instruct".to_string(),
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

    Ok(response.choices[0].message.content.trim().to_string())
}
