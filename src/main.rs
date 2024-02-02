use std::{env, sync::Arc};

use aws_config::{meta::region::RegionProviderChain, BehaviorVersion};
use lambda_http::{run, service_fn, Error};
use teloxide::Bot;
use tokio::sync::Mutex;
use tracing::error;
use tracing_subscriber::fmt;
use utils::set_commands;

mod dynamodb;
mod openai;
mod telegram;
mod utils;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing for logging
    fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .without_time()
        .init();

    // Setup telegram bot (we do it here because this place is executed less often)
    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    // Update the bot's commands
    if let Err(err) = set_commands(&bot).await {
        error!("Failed to set commands: {}", err);
    }

    // Arc and mutex for thread safety
    let bot = Arc::new(Mutex::new(bot));

    // Setup the dynamodb client
    let region_provider: RegionProviderChain =
        RegionProviderChain::default_provider().or_else("eu-central-1");
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;

    let dynamodb_client = aws_sdk_dynamodb::Client::new(&config);
    // Arc and mutex for thread safety
    let dynamodb_client = Arc::new(Mutex::new(dynamodb_client));

    // Run the Lambda function
    run(service_fn(|req| {
        telegram::handle_telegram_request(req, Arc::clone(&bot), Arc::clone(&dynamodb_client))
    }))
    .await
}
