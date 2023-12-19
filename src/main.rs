use lambda_http::{
    aws_lambda_events, http::HeaderMap, run, service_fn, Body, Error, Request, Response,
};
use reqwest::header::AUTHORIZATION;
use serde_json::json;
use std::env;
use teloxide::{
    requests::Requester,
    types::{Update, UpdateKind},
    Bot,
};
use tracing::info;

async fn convert_input_to_json(input: Request) -> Result<Update, Error> {
    let body = input.body();
    let body_str = match body {
        Body::Text(text) => text,
        not => panic!("expected Body::Text(...) got {:?}", not),
    };
    let body_json: Update = serde_json::from_str(body_str).unwrap();
    Ok(body_json)
}

async fn handle_telegram_request(req: Request) -> Result<Response<Body>, Error> {
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    let update = convert_input_to_json(req).await.unwrap();
    info!("update: {:?}", update);

    // Match the update type
    match update.kind {
        // If the update is a message
        UpdateKind::Message(message) => {
            // If the message is a text message

            bot.send_message(message.chat.id, "Hello, world!")
                .await
                .unwrap();
        }
        _ => {}
    }

    return Ok(Response::builder()
        .status(200)
        .body(Body::from("Hello, world!"))
        .unwrap());
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing for logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .without_time()
        .init();

    // Run the Lambda function
    run(service_fn(handle_telegram_request)).await
}
