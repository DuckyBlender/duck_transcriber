use crate::telegram::TELEGRAM_OWNER_ID;
use anyhow::Result;
use lambda_http::{Body, Request};
use teloxide::types::{Message, Update};

// anyhow result
pub async fn convert_input_to_json(input: Request) -> Result<Update> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        not => panic!("expected Body::Text(...) got {not:?}"),
    };
    let body_json: Update = serde_json::from_str(body_str)?;
    Ok(body_json)
}

pub async fn is_owner(message: &Message) -> Result<bool> {
    // Get the user ID
    let user_id = message.from().as_ref().unwrap().id;
    if user_id == teloxide::prelude::UserId(TELEGRAM_OWNER_ID) {
        Ok(true)
    } else {
        Ok(false)
    }
}
