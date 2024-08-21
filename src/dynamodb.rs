use std::env;

use aws_sdk_dynamodb::{types::AttributeValue, Client, Error};
use aws_sdk_kms::primitives::Blob;
use tracing::info;

pub struct DBItem {
    pub transcription: Blob, // encrypted bytes
    pub file_id: String,
    pub unix_timestamp: i64,
}

pub async fn get_item(client: &Client, file_id: &String) -> Result<Option<Blob>, Error> {
    let table = env::var("DYNAMODB_TABLE").unwrap();
    let key = AttributeValue::S(file_id.to_string());

    info!(
        "Querying DynamoDB table '{}' for file_id '{}'",
        table, file_id
    );

    let results = client
        .query()
        .table_name(table)
        .key_condition_expression("#fileId = :fileId")
        .expression_attribute_names("#fileId", "fileId")
        .expression_attribute_values(":fileId", key)
        .limit(1)
        .send()
        .await?;

    if let Some(item) = results.items {
        if item.is_empty() {
            info!("No items found for file_id '{}'", file_id);
            return Ok(None);
        }

        let transcription = item
            .first()
            .unwrap()
            .get("transcription")
            .unwrap()
            .as_b()
            .unwrap()
            .to_owned();

        info!("Transcription found for file_id '{}'", file_id);
        Ok(Some(transcription))
    } else {
        info!("No items found for file_id '{}'", file_id);
        Ok(None)
    }
}

pub async fn get_item_count(client: &Client) -> Result<i32, Error> {
    let table = env::var("DYNAMODB_TABLE").unwrap();

    info!("Querying DynamoDB table '{}' for item count", table);

    let results = client
        .scan()
        .table_name(&table)
        .select("COUNT".into())
        .send()
        .await?;

    let count = results.count;

    info!("Item count for table '{}': {}", &table, count);
    Ok(count)
}

pub async fn add_item(client: &Client, item: DBItem) -> Result<(), Error> {
    let table = env::var("DYNAMODB_TABLE").unwrap();
    let transcription = AttributeValue::B(item.transcription);
    let file_id = AttributeValue::S(item.file_id);
    let unix_timestamp = AttributeValue::N(item.unix_timestamp.to_string());

    client
        .put_item()
        .table_name(table)
        .item("transcription", transcription)
        .item("fileId", file_id)
        .item("unixTimestamp", unix_timestamp)
        .send()
        .await?;

    Ok(())
}
