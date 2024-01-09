use lambda_http::{run, service_fn, Error};
use tracing_subscriber::fmt;

mod openai;
mod telegram;
mod utils;
mod bedrock;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing for logging
    fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .without_time()
        .init();

    // Run the Lambda function
    run(service_fn(telegram::handle_telegram_request)).await
}
