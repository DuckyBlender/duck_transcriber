use std::env;

use reqwest::header::AUTHORIZATION;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::BASE_URL;

#[derive(Debug, Serialize)]
struct GroqChatRequest {
    model: String,
    messages: Vec<GroqChatMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct GroqChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct GroqChatResponse {
    choices: Vec<GroqChatChoice>,
}

#[derive(Debug, Deserialize)]
struct GroqChatChoice {
    message: GroqChatMessage,
}

pub enum SummarizeMethod {
    Default,
    Caveman,
}

pub async fn summarize(text: &str, method: SummarizeMethod) -> Result<String, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!(
            "Bearer {}",
            env::var("GROQ_API_KEY").expect("GROQ_API_KEY not found")
        )
        .parse()
        .unwrap(),
    );

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
        model: "meta-llama/llama-4-scout-17b-16e-instruct".to_string(),
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
        .post(format!("{}/chat/completions", BASE_URL))
        .headers(headers)
        .json(&request)
        .send()
        .await
        .map_err(|err| format!("Failed to send request to Groq: {}", err))?;

    if !res.status().is_success() {
        let json = res
            .json::<serde_json::Value>()
            .await
            .map_err(|err| format!("Failed to parse Groq error response: {err}"))?;
        return Err(format!(
            "Groq returned an error: {}",
            json["error"]["message"]
        ));
    }

    let response = res
        .json::<GroqChatResponse>()
        .await
        .map_err(|err| format!("Failed to parse Groq response: {}", err))?;

    Ok(response.choices[0].message.content.trim().to_string())
}
