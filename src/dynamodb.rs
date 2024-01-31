use std::collections::HashMap;

use aws_sdk_dynamodb::types::{AttributeValue, Select};
use teloxide::types::UserId;
use tracing::info;

use crate::telegram::TranscriptionData;

pub async fn insert_data(
    dynamodb_client: &aws_sdk_dynamodb::Client,
    transcription_data: TranscriptionData,
) -> Result<(), aws_sdk_dynamodb::Error> {
    let mut item = HashMap::new();
    item.insert(
        "userId".to_string(),
        AttributeValue::N(transcription_data.user_id.to_string()),
    );
    item.insert(
        "timestamp".to_string(),
        AttributeValue::S(transcription_data.timestamp),
    );
    item.insert(
        "secondsTranscribed".to_string(),
        AttributeValue::N(transcription_data.seconds_transcribed.to_string()),
    );

    let table_name = "duck_transcriber_stats";
    let put_req = dynamodb_client
        .put_item()
        .table_name(table_name)
        .set_item(Some(item))
        .send()
        .await?;

    info!("Put item: {:?}", put_req);
    Ok(())
}

// /stats command
// sends a message with the stats (top 5 people with most seconds uploaded, top 5 people with most seconds transcribed and the user's stats with his rank)
// Continuing the stats function
pub async fn stats(
    dynamodb_client: &aws_sdk_dynamodb::Client,
    user_id: UserId,
) -> Result<String, aws_sdk_dynamodb::Error> {
    // Assuming a simple table structure, adjust as necessary
    let scan_output = dynamodb_client
        .scan()
        .table_name("duck_transcriber_stats")
        .select(Select::AllAttributes)
        .send()
        .await?;

    let mut user_stats = HashMap::new();
    let mut all_stats = Vec::new();

    if let Some(items) = scan_output.items {
        for item in items {
            let user_id_str = item
                .get("userId")
                .map(|v| v.as_n())
                .unwrap()
                .unwrap()
                .to_string();
            let seconds_transcribed = item
                .get("secondsTranscribed")
                .map(|v| v.as_n())
                .unwrap()
                .unwrap()
                .parse::<i32>()
                .unwrap_or(0);

            // Collect stats for all users
            let entry = user_stats.entry(user_id_str.clone()).or_insert(0);
            *entry += seconds_transcribed;

            // Check if this is the specific user we're looking for
            if user_id_str == user_id.to_string() {
                all_stats.push((user_id_str, seconds_transcribed));
            }
        }
    }

    // Sort and find top 5
    let mut sorted_stats: Vec<_> = user_stats.into_iter().collect();
    sorted_stats.sort_by(|a, b| b.1.cmp(&a.1));
    let top_5 = sorted_stats.into_iter().take(5);

    // Find user's rank
    let user_rank = all_stats
        .iter()
        .position(|(id, _)| id == &user_id.to_string())
        .map(|pos| pos + 1)
        .unwrap_or(0);

    // Format the message as HTML
    let mut message = String::from("<b>Top 5 Users by Seconds Transcribed:</b>\n");
    for (rank, (user, seconds)) in top_5.enumerate() {
        message.push_str(&format!(
            // <a href="tg://user?id=123456789">inline mention of a user</a>
            "{}. <a href=\"tg://user?id={}\">{}</a>: <code>{}</code>\n",
            rank + 1,
            user,
            user,
            seconds
        ));
    }

    // Add user's stats and rank
    if let Some((_, user_seconds)) = all_stats
        .into_iter()
        .find(|(id, _)| id == &user_id.to_string())
    {
        message.push_str(&format!(
            "\n<b>Your Stats:</b>\nSeconds Transcribed: <code>{}</code>\nYour Rank: <code>{}</code>",
            user_seconds, user_rank
        ));
    } else {
        message.push_str("\n<b>Your Stats:</b>\n<code>No data available.</code>");
    }

    Ok(message)
}
