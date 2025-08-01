use aws_sdk_dynamodb::{Client, Error, types::AttributeValue};
use log::info;
use std::env;
use teloxide::types::FileUniqueId;

use crate::transcribe::TaskType;

pub struct DBItem {
    pub text: String,
    pub unique_file_id: String, // Using String for compatibility with DynamoDB
    pub task_type: String,
}

pub enum ItemReturnInfo {
    Text(String),
    Exists, // Item already exists, but for other task type.
    None,
}

pub async fn get_item(
    client: &Client,
    unique_file_id: &FileUniqueId,
    task_type: &TaskType,
) -> Result<ItemReturnInfo, Error> {
    let table = env::var("DYNAMODB_TABLE").unwrap();
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

    if let Some(item) = results.items {
        if item.is_empty() {
            info!("No items found for unique_file_id '{unique_file_id}'");
            return Ok(ItemReturnInfo::None);
        }

        let transcription = item.first().unwrap().get(&task_type);

        match transcription {
            Some(transcription) => {
                info!("{task_type} found for unique_file_id '{unique_file_id}'");
                let transcription = transcription.as_s().unwrap().to_string();
                Ok(ItemReturnInfo::Text(transcription))
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
    let table = env::var("DYNAMODB_TABLE").unwrap();
    let key = AttributeValue::S(unique_file_id.to_string());
    let task_type = task_type.to_string();
    let text = AttributeValue::S(text.to_string());

    info!("Updating DynamoDB table '{table}' for unique_file_id '{unique_file_id}'");

    client
        .update_item()
        .table_name(table)
        .key("id", key)
        .update_expression(format!("SET #{task_type} = :text"))
        .expression_attribute_names(format!("#{task_type}"), task_type)
        .expression_attribute_values(":text", text)
        .send()
        .await?;

    Ok(())
}

pub async fn add_item(client: &Client, item: DBItem) -> Result<(), Error> {
    let table = env::var("DYNAMODB_TABLE").unwrap();
    let text = AttributeValue::S(item.text);
    let file_id = AttributeValue::S(item.unique_file_id);

    client
        .put_item()
        .table_name(table)
        .item(item.task_type, text)
        .item("id", file_id)
        .send()
        .await?;

    Ok(())
}
