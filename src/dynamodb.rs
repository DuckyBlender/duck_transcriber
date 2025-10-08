use crate::types::{DBItem, ItemReturnInfo, TaskType};
use aws_sdk_dynamodb::{Client, Error, types::AttributeValue};
use log::{error, info};
use std::env;
use teloxide::types::FileUniqueId;

const EXPIRATION_DAYS: i64 = 7;

fn get_table_name() -> String {
    env::var("DYNAMODB_TABLE").unwrap_or_else(|_| {
        error!("DYNAMODB_TABLE environment variable not set");
        panic!("DYNAMODB_TABLE must be configured");
    })
}

pub async fn get_item(
    client: &Client,
    unique_file_id: &FileUniqueId,
    task_type: &TaskType,
) -> Result<ItemReturnInfo, Error> {
    let table = get_table_name();
    let key = AttributeValue::S(unique_file_id.to_string());
    let task_type = task_type.to_string();

    info!("Querying DynamoDB table '{table}' for unique_file_id '{unique_file_id}'");

    let results = client
        .query()
        .table_name(table)
        .key_condition_expression("#id = :id")
        .expression_attribute_names("#id", "id")
        .expression_attribute_values(":id", key)
        .limit(1)
        .send()
        .await?;

    if let Some(items) = results.items {
        if items.is_empty() {
            info!("No items found for unique_file_id '{unique_file_id}'");
            return Ok(ItemReturnInfo::None);
        }

        let first_item = match items.first() {
            Some(item) => item,
            None => {
                error!("Failed to get first item from DynamoDB results");
                return Ok(ItemReturnInfo::None);
            }
        };

        match first_item.get(&task_type) {
            Some(transcription) => {
                info!("{task_type} found for unique_file_id '{unique_file_id}'");
                match transcription.as_s() {
                    Ok(text) => Ok(ItemReturnInfo::Text(text.to_string())),
                    Err(e) => {
                        error!("Failed to parse transcription as string: {e:?}");
                        Ok(ItemReturnInfo::None)
                    }
                }
            }
            None => {
                info!("No {task_type} found for unique_file_id '{unique_file_id}'");
                Ok(ItemReturnInfo::Exists)
            }
        }
    } else {
        info!("No items found for unique_file_id '{unique_file_id}'");
        Ok(ItemReturnInfo::None)
    }
}

pub async fn append_attribute(
    client: &Client,
    unique_file_id: &FileUniqueId,
    task_type: &TaskType,
    text: &String,
) -> Result<(), Error> {
    let table = get_table_name();
    let key = AttributeValue::S(unique_file_id.to_string());
    let task_type = task_type.to_string();
    let text = AttributeValue::S(text.to_string());
    let expires_at = AttributeValue::N(
        (chrono::Utc::now() + chrono::Duration::days(EXPIRATION_DAYS))
            .timestamp()
            .to_string(),
    );

    info!("Updating DynamoDB table '{table}' for unique_file_id '{unique_file_id}'");

    client
        .update_item()
        .table_name(table)
        .key("id", key)
        .update_expression(format!(
            "SET #{task_type} = :text, expires_at = :expires_at"
        ))
        .expression_attribute_names(format!("#{task_type}"), task_type)
        .expression_attribute_values(":text", text)
        .expression_attribute_values(":expires_at", expires_at)
        .send()
        .await?;

    Ok(())
}

pub async fn add_item(client: &Client, item: DBItem) -> Result<(), Error> {
    let table = get_table_name();
    let text = AttributeValue::S(item.text);
    let file_id = AttributeValue::S(item.unique_file_id);
    let expires_at = AttributeValue::N(item.expires_at.to_string());

    client
        .put_item()
        .table_name(table)
        .item(item.task_type, text)
        .item("id", file_id)
        .item("expires_at", expires_at)
        .send()
        .await?;

    Ok(())
}
