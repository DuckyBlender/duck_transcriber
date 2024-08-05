use std::env;

use aws_sdk_dynamodb::{types::AttributeValue, Client, Error};
use tracing::{debug, info};

pub struct Item {
    pub transcription: String,
    pub file_id: String,
    pub unix_timestamp: String,
}

pub async fn get_item(client: &Client, file_id: &String) -> Result<Option<String>, Error> {
    let table = env::var("DYNAMODB_TABLE").unwrap();
    let key = AttributeValue::S(file_id.to_string());

    info!("Querying DynamoDB table '{}' for file_id '{}'", table, file_id);

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
            .as_s()
            .as_ref()
            .unwrap()
            .to_string();

            info!("Transcription found for file_id '{}': {}", file_id, transcription);
            Ok(Some(transcription))
        } else {
            info!("No items found for file_id '{}'", file_id);
            Ok(None)
        }
}

pub async fn add_item(client: &Client, item: Item) -> Result<(), Error> {
    let table = env::var("DYNAMODB_TABLE").unwrap();
    let transcription = AttributeValue::S(item.transcription);
    let file_id = AttributeValue::S(item.file_id);
    let unix_timestamp = AttributeValue::S(item.unix_timestamp);

    let resp = client
        .put_item()
        .table_name(table)
        .item("transcription", transcription)
        .item("fileId", file_id)
        .item("unixTimestamp", unix_timestamp)
        .send()
        .await?;

    debug!("Response: {:?}", resp);

    Ok(())
}
