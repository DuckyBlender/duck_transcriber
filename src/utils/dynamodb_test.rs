use aws_sdk_dynamodb::operation::put_item::PutItemError;
use aws_sdk_dynamodb::operation::update_item::UpdateItemError;
use aws_sdk_dynamodb::types::ReturnValue;
use aws_sdk_dynamodb::{error::SdkError, types::AttributeValue};
use aws_sdk_dynamodb::Client;
use tracing::{error, info};

pub const TABLE_NAME: &str = "duck_transcriber_db";

#[derive(Clone, Debug)]
pub struct Item {
    pub table: String,
    pub user_id: u64,
    pub transcribed_seconds: u64,
}

async fn add_item(client: &Client, item: Item) -> Result<(), SdkError<PutItemError>> {
    info!("Adding item to table");
    info!("item: {:?}", item);

    let mut item_map = std::collections::HashMap::new();
    item_map.insert(
        "userId".to_string(),
        AttributeValue::N(item.user_id.to_string()),
    );
    item_map.insert(
        "transcribedSeconds".to_string(),
        AttributeValue::N(item.transcribed_seconds.to_string()),
    );

    match client
        .put_item()
        .table_name(&item.table)
        .set_item(Some(item_map))
        .send()
        .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

pub async fn query_item(client: &Client, item: Item) -> Option<u64> {
    info!("Querying table for item");

    let key = std::collections::HashMap::from([(
        "userId".to_string(),
        AttributeValue::N(item.user_id.to_string()),
    )]);

    match client.get_item().table_name(&item.table).set_key(Some(key)).send().await {
        Ok(resp) => {
            if let Some(item) = resp.item() {
                info!("Found a matching entry in the table:");
                let seconds_transcribed = item
                    .get("transcribedSeconds").map(|v| v.as_n())
                    .and_then(|n| n.unwrap().parse::<u64>().ok());

                seconds_transcribed
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

async fn update_seconds(client: &Client, item: Item) -> Result<(), SdkError<UpdateItemError>> {
    info!("Updating seconds item in table");

    let key = std::collections::HashMap::from([(
        "userId".to_string(),
        AttributeValue::N(item.user_id.to_string()),
    )]);

    let update_expression = "SET transcribedSeconds = transcribedSeconds + :val".to_string();
    let expression_attribute_values = std::collections::HashMap::from([(
        ":val".to_string(),
        AttributeValue::N(item.transcribed_seconds.to_string()),
    )]);

    match client
        .update_item()
        .table_name(&item.table)
        .set_key(Some(key))
        .update_expression(update_expression)
        .set_expression_attribute_values(Some(expression_attribute_values))
        .return_values(ReturnValue::None)
        .send()
        .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

pub async fn smart_add_item(client: &Client, item: Item) -> Result<(), SdkError<()>> {
    info!("Checking if user {} exists in table, and adding if not", item.user_id);
    if query_item(client, item.clone()).await.is_some() {
        info!("Updating seconds, because user {} already exists", item.user_id);
        let _res = update_seconds(client, item).await;
        // TODO: Handle this more nicely
        Ok(())

    } else {
        info!("Adding item, because user {} does not exist", item.user_id);
        let _res = add_item(client, item).await;
        Ok(())
    }
}