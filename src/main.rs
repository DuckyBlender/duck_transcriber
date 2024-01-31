use std::{env, sync::Arc};

use lambda_http::{run, service_fn, Error};
use teloxide::Bot;
use tokio::sync::Mutex;
use tracing_subscriber::fmt;

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

    let bot = Bot::new(env::var("TELEGRAM_BOT_TOKEN").unwrap());
    // arc and mutex for thread safety
    let bot = Arc::new(Mutex::new(bot));

    // Run the Lambda function
    run(service_fn(|req| {
        telegram::handle_telegram_request(req, Arc::clone(&bot))
    }))
    .await
}
