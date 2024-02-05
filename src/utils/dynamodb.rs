use crate::utils::other::TranscriptionData;
use aws_sdk_dynamodb::types::AttributeValue;
use std::{collections::HashMap, env};
use teloxide::types::UserId;
use tracing::{error, info};

pub async fn insert_data(
    dynamodb_client: &aws_sdk_dynamodb::Client,
    transcription_data: TranscriptionData,
) -> Result<(), aws_sdk_dynamodb::Error> {
    let table_name = env::var("DYNAMODB_TABLE_NAME").unwrap();

    // Check if the user is in the database
    let get_item_output = dynamodb_client
        .get_item()
        .table_name(&table_name)
        .key(
            "userId",
            AttributeValue::N(transcription_data.user_id.to_string()),
        )
        .send()
        .await?;

    let mut new_seconds_transcribed = transcription_data.seconds_transcribed;

    // If the user is in the database, update the user's seconds_transcribed
    if get_item_output.item.is_some() {
        info!("User is in the database, getting user's seconds_transcribed");
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
        new_seconds_transcribed += current_seconds_transcribed;
        info!(
            "User's current seconds_transcribed: {}",
            current_seconds_transcribed
        );
    }

    // Insert or update the user's seconds_transcribed
    info!("Updating user's seconds_transcribed");
    let put_req = dynamodb_client
        .put_item()
        .table_name(&table_name)
        .item(
            "userId",
            AttributeValue::N(transcription_data.user_id.to_string()),
        )
        .item(
            "transcribedSeconds",
            AttributeValue::N(new_seconds_transcribed.to_string()),
        )
        .send()
        .await
        .map_err(|e| {
            error!("Failed to update user's seconds_transcribed: {}", e);
            error!("DEBUG: {:?}", e);
            e
        })?;

    info!(
        "User {} seconds_transcribed updated to {}",
        transcription_data.user_id, new_seconds_transcribed
    );

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
    let query_output = dynamodb_client
        .query()
        .table_name(env::var("DYNAMODB_TABLE_NAME").unwrap())
        .key_condition_expression("#userId = :userIdVal")
        .expression_attribute_names("#userId", "userId")
        .expression_attribute_values(":userIdVal", AttributeValue::N(user_id.to_string()))
        .send()
        .await?;

    if let Some(items) = query_output.items {
        info!("User is in the database!");
        if let Some(first_item) = items.first() {
            info!("Getting user's seconds_transcribed");
            if let Some(transcribed_seconds) = first_item.get("transcribedSeconds") {
                info!("User's seconds found");
                if let Ok(n) = transcribed_seconds.as_n() {
                    info!("User's seconds: {}", n);
                    if let Ok(total_transcribed) = n.parse::<i64>() {
                        info!("User's seconds parsed: {}", total_transcribed);
                        let message = format!(
                            "<b>{}'s Stats</b>\nSeconds Transcribed: <code>{}</code>",
                            username, total_transcribed
                        );
                        return Ok(message);
                    }
                }
            }
        }
    }

    info!("User is not in the database");
    Ok("You don't have any stats yet. Transcribe something to get started!".to_string())
}
