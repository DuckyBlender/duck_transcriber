use lambda_http::{run, service_fn, Body, Error, Request, RequestExt, Response};
use sqlx::{mysql::MySqlPoolOptions, Row};
use std::sync::Arc;
use std::env;

async fn function_handler(event: Request, pool: Arc<sqlx::Pool<sqlx::MySql>>) -> Result<Response<Body>, Error> {
    // Extract some useful information from the request
    // For example, list all database tables
    let rows = sqlx::query("SHOW DATABASES")
        .fetch_all(&*pool)
        .await?;

    let mut message = String::from("Tables in the database:\n");
    for row in rows {
        let table_name: String = row.get(0);
        message.push_str(&format!("{}\n", table_name));
    }

    // Return something that implements IntoResponse.
    // It will be serialized to the right response event automatically by the runtime
    let resp = Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body(message.into())
        .map_err(Box::new)?;
    Ok(resp)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Setup a database connection pool
    let user = env::var("DB_USER").expect("DB_USER not set");
    let pass = env::var("DB_PASS").expect("DB_PASS not set");
    let url = env::var("DB_URL").expect("DB_URL not set");
    let port = env::var("DB_PORT").expect("DB_PORT not set");
    let pool = MySqlPoolOptions::new()
        .max_connections(5)
        .connect(format!("mysql://{user}:{pass}@{url}:{port}").as_str())
        .await
        .expect("Failed to connect to MySQL DB");

    let pool = Arc::new(pool);

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        // disable printing the name of the module in every log line.
        .with_target(false)
        // disabling time is handy because CloudWatch will add the ingestion time.
        .without_time()
        .init();

    run(service_fn(move |req| function_handler(req, pool.clone()))).await
}