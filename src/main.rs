use std::{env, sync::Arc};

use lambda_http::{run, service_fn, Error};
use sqlx::mysql::MySqlPoolOptions;
use telegram::handle_telegram_request;
use tracing_subscriber::fmt;

mod openai;
mod sql;
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

    // Initialize the database connection pool
    // This needs to be done here for performance reasons
    let user = env::var("DB_USER").expect("DB_USER not set");
    let pass = env::var("DB_PASS").expect("DB_PASS not set");
    let url = env::var("DB_URL").expect("DB_URL not set");
    let port = env::var("DB_PORT").expect("DB_PORT not set");
    let pool = MySqlPoolOptions::new()
        .max_connections(5)
        .connect(format!("mysql://{user}:{pass}@{url}:{port}/transcriber").as_str())
        .await
        .expect("Failed to connect to MySQL DB");

    // Arc is used to share the pool across threads
    let pool = Arc::new(pool);

    // Run the Lambda function
    run(service_fn(move |req| {
        handle_telegram_request(req, pool.clone())
    }))
    .await
}
