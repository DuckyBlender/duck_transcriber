use std::env;

use reqwest::header::HeaderMap;
use reqwest::header::AUTHORIZATION;
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

pub async fn summarize(text: &str) -> Result<String, String> {
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

    let request = GroqChatRequest {
        model: "llama-3.1-8b-instant".to_string(),
        messages: vec![
            GroqChatMessage {
                role: "system".to_string(),
                content: "You are AI that summarizes transcriptions of voice messages. Make the summarization language in the transcription language. Make the summary concise but comprehensive. If you're not sure what is in the voice message or it doesn't make sense, just output three question marks.".to_string(),
            },
            GroqChatMessage {
                role: "user".to_string(),
                content: text.to_string(),
            },
        ],
        temperature: 0.7,
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
