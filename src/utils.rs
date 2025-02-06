use lambda_http::{Body, Request};
use serde_json::Error;
use teloxide::types::Update;

pub fn split_string(input: &str, max_length: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_chunk = String::new();
    let mut current_length = 0;

    for word in input.split_whitespace() {
        if current_length + word.len() + 1 > max_length && !current_chunk.is_empty() {
            result.push(current_chunk);
            current_chunk = String::new();
            current_length = 0;
        }

        if current_length > 0 {
            current_chunk.push(' ');
            current_length += 1;
        }

        current_chunk.push_str(word);
        current_length += word.len();
    }

    if !current_chunk.is_empty() {
        result.push(current_chunk);
    }

    result
}

pub async fn parse_webhook(input: Request) -> Result<Update, Error> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        not => panic!("expected Body::Text(...) got {not:?}"),
    };
    let body_json: Update = serde_json::from_str(body_str)?;
    Ok(body_json)
}
