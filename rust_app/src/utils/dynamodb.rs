// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::result_large_err)]

use aws_sdk_dynamodb::error::SdkError;

use aws_sdk_dynamodb::operation::execute_statement::ExecuteStatementError;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client;
use tracing::{error, info};

pub const TABLE_NAME: &str = "duck-transcriber-db";

/// A struct for the arguments and returns from add_item and query_item.
#[derive(Clone, Debug)]
pub struct Item {
    pub table: String,
    pub user_id: u64,
    pub transcribed_seconds: u64,
}

/// Add an item to the table.
async fn add_item(client: &Client, item: Item) -> Result<(), SdkError<ExecuteStatementError>> {
    info!("Adding item to table: {:?}", item);
    match client
        .execute_statement()
        .statement(format!(
            r#"INSERT INTO "{}" VALUE {{
                'userId': ?,
                'transcribedSeconds': ?
        }}"#,
            item.table,
        ))
        .set_parameters(Some(vec![
            AttributeValue::N(item.user_id.to_string()),
            AttributeValue::N(item.transcribed_seconds.to_string()),
        ]))
        .send()
        .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Query the table for an item matching the input values.
/// Returns None if no matching item is found or Some(u64) if a matching item is found.
pub async fn query_item(client: &Client, item: Item) -> Option<u64> {
    info!("Querying table for item");
    match client
        .execute_statement()
        .statement(format!(
            r#"SELECT * FROM "{}" WHERE userId = ?"#,
            item.table
        ))
        .set_parameters(Some(vec![AttributeValue::N(item.user_id.to_string())]))
        .send()
        .await
    {
        Ok(resp) => {
            if !resp.items().is_empty() {
                info!("Found a matching entry in the table:");
                let seconds_transcribed = resp
                    .items
                    .unwrap_or_default()
                    .pop()
                    .unwrap()
                    .get("transcribedSeconds")
                    .unwrap()
                    .as_n()
                    .unwrap()
                    .parse::<u64>()
                    .unwrap();

                Some(seconds_transcribed)
            } else {
                info!("Did not find a match.");
                None
            }
        }
        Err(e) => {
            error!("Got an error querying table:");
            error!("{}", e);
            None
        }
    }
}

async fn update_seconds(
    client: &Client,
    item: Item,
) -> Result<(), SdkError<ExecuteStatementError>> {
    info!("Updating seconds item in table");
    match client
        .execute_statement()
        .statement(format!(
            r#"UPDATE "{}" SET transcribedSeconds = transcribedSeconds + ? WHERE userId = ?"#,
            item.table
        ))
        .set_parameters(Some(vec![
            AttributeValue::N(item.transcribed_seconds.to_string()),
            AttributeValue::N(item.user_id.to_string()),
        ]))
        .send()
        .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}
// First query the table to see if the item exists. If it does, update it; otherwise, add it.
pub async fn smart_add_item(
    client: &Client,
    item: Item,
) -> Result<(), SdkError<ExecuteStatementError>> {
    info!(
        "Checking if user {} exists in table, and adding if not",
        item.user_id
    );
    if (query_item(client, item.clone()).await).is_some() {
        info!(
            "Updating seconds, because user {} already exists",
            item.user_id
        );
        update_seconds(client, item).await
    } else {
        info!("Adding item, because user {} does not exist", item.user_id);
        add_item(client, item).await
    }
}

// Deletes an item from a table.
// async fn remove_item(client: &Client, table: &str, key: &str, value: String) -> Result<(), Error> {
//     client
//         .execute_statement()
//         .statement(format!(r#"DELETE FROM "{table}" WHERE "{key}" = ?"#))
//         .set_parameters(Some(vec![AttributeValue::N(value)]))
//         .send()
//         .await?;

//     info!("Deleted item.");

//     Ok(())
// }
