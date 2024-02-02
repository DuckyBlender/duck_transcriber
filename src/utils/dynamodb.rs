use crate::utils::other::TranscriptionData;
use aws_sdk_dynamodb::{operation::query, types::AttributeValue};
use std::{collections::HashMap, env};
use teloxide::types::UserId;
use tracing::info;

pub async fn insert_data(
    dynamodb_client: &aws_sdk_dynamodb::Client,
    transcription_data: TranscriptionData,
) -> Result<(), aws_sdk_dynamodb::Error> {
    // Check if the user is in the database
    // If the user is in the database, update the user's seconds_transcribed
    // If the user is not in the database, add the user to the database
    let get_item_output = dynamodb_client
        .get_item()
        .table_name(env::var("DYNAMODB_TABLE_NAME").unwrap())
        .key(
            "userId",
            AttributeValue::N(transcription_data.user_id.to_string()),
        )
        .send()
        .await?;

    if get_item_output.item.is_some() {
        info!("User is in the database");
        // Update the user's seconds_transcribed
        // First get the user's current seconds_transcribed
        let current_seconds_transcribed = get_item_output
            .item
            .unwrap()
            .get("transcribedSeconds")
            .unwrap()
            .as_n()
            .unwrap()
            .parse::<i64>()
            .unwrap();

        // Add the new seconds_transcribed to the user's current seconds_transcribed
        let new_seconds_transcribed =
            current_seconds_transcribed + transcription_data.seconds_transcribed;

        // Update the user's seconds_transcribed
        let mut item = HashMap::new();
        item.insert(
            "userId".to_string(),
            AttributeValue::N(transcription_data.user_id.to_string()),
        );
        item.insert(
            "transcribedSeconds".to_string(),
            AttributeValue::N(new_seconds_transcribed.to_string()),
        );
        // Update the user's seconds_transcribed
        let put_req = dynamodb_client
            .put_item()
            .table_name(env::var("DYNAMODB_TABLE_NAME").unwrap())
            .set_item(Some(item))
            .send()
            .await?;

        info!(
            "User {} seconds_transcribed updated to {}",
            transcription_data.user_id, new_seconds_transcribed
        );
    } else {
        info!("User is not in the database");
        // Add them to the database
        let mut item = HashMap::new();
        item.insert(
            "userId".to_string(),
            AttributeValue::N(transcription_data.user_id.to_string()),
        );
        item.insert(
            "transcribedSeconds".to_string(),
            AttributeValue::N(transcription_data.seconds_transcribed.to_string()),
        );

        let put_req = dynamodb_client
            .put_item()
            .table_name(env::var("DYNAMODB_TABLE_NAME").unwrap())
            .set_item(Some(item))
            .send()
            .await?;
        info!(
            "User {} to the database with {} seconds",
            transcription_data.user_id, transcription_data.seconds_transcribed
        );
    }

    Ok(())
}

// /stats command
// Sends a message with the stats
// Here is the command output:
// Your Stats:
// Seconds Transcribed: <code>123</code> ([user's rank]st/nd/rd/th)
// Continuing the stats function
pub async fn stats(
    dynamodb_client: &aws_sdk_dynamodb::Client,
    user_id: UserId,
    username: String,
) -> Result<String, aws_sdk_dynamodb::Error> {
    // Query the user's stats
    let query_output = dynamodb_client
        .query()
        .table_name(env::var("DYNAMODB_TABLE_NAME").unwrap())
        .key_condition_expression("#userId = :userIdVal")
        .expression_attribute_names("#userId", "userId")
        .expression_attribute_values(":userIdVal", AttributeValue::N(user_id.to_string()))
        .send()
        .await?;

    let total_transcribed: i64;

    // If the user is not in the database, tell the user that they are not in the database
    if query_output.items.is_none() {
        info!("User is not in the database");
        // Return "USER IS NOT IN DATABASE"
        Ok("You don't have any stats yet. Transcribe something to get started!".to_string())
    } else {
        // If the user is in the database, get the user's stats
        info!("User is in the database");
        // Get the user's stats
        total_transcribed = query_output
            .items
            .unwrap()
            .first()
            .unwrap()
            .get("transcribedSeconds")
            .unwrap()
            .as_n()
            .unwrap()
            .parse::<i64>()
            .unwrap();

        // TODO: Get the user's rank

        let message = format!(
            "<b>{}'s Stats\nSeconds Transcribed: <code>{}</code>",
            username, total_transcribed
        );
        Ok(message)
    }
}
