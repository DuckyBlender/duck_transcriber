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

            let entry = user_stats.entry(user_id_str.clone()).or_insert(0);
            *entry += seconds_transcribed;

            all_stats.push((user_id_str.clone(), seconds_transcribed));
        }
    }

    all_stats.sort_by(|a, b| b.1.cmp(&a.1));

    let user_name = user_id.to_string();
    let mut user_rank = 0;
    for (index, (user, _)) in all_stats.iter().enumerate() {
        if user == &user_name {
            user_rank = index + 1;
            break;
        }
    }
    if user_rank == 0 {
        return Ok("You have no stats yet! Try sending a voice message or video note.".to_string());
    }

    let ordinal = match user_rank {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    };

    let message = format!(
        "<b>Stats for: {}</b>\nSeconds Transcribed: <code>{}</code> ({}{})",
        username,
        user_stats.get(&user_name).unwrap_or(&0),
        user_rank,
        ordinal
    );

    Ok(message)
}
