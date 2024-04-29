use std::env;

use aws_config::{meta::region::RegionProviderChain, BehaviorVersion};
use lambda_runtime::{run, service_fn, Error};

use serde::Serialize;
use teloxide::Bot;
use tracing::error;
use utils::{other::set_commands, telegram::handle_telegram_request};

mod commands;
mod listeners;
mod utils;

#[derive(Serialize)]
struct Response {
    body: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        // disable printing the name of the module in every log line.
        .with_target(false)
        // disabling time is handy because CloudWatch will add the ingestion time.
        .without_time()
        .init();

    // Setup telegram bot (we do it here because this place is a cold start)
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    // Update the bot's commands
    if let Err(err) = set_commands(&bot).await {
        error!("Failed to set commands: {}", err);
    }

    // Setup the dynamodb client
    let region_provider: RegionProviderChain =
        RegionProviderChain::default_provider().or_else("eu-central-1");
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    let dynamodb_client = aws_sdk_dynamodb::Client::new(&config);

    run(service_fn(|req| {
        handle_telegram_request(req, &bot, &dynamodb_client)
    }))
    .await
}
