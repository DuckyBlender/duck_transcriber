use aws_sdk_bedrockruntime as bedrockruntime;
use bedrockruntime::{config::endpoint::ParamsBuilder, primitives::Blob};
use serde_json::json;
use base64::decode;
use std::error::Error;

pub async fn generate_image(prompt: String) -> Result<Vec<u8>, Box<dyn Error>> {
    // Create a Bedrock Runtime client
    let config = aws_config::load_from_env().await;
    let client = aws_sdk_bedrockruntime::Client::new(&config);

    let input = json!({
    "taskType": "TEXT_IMAGE",
    "textToImageParams": {
        "text": prompt
    },
    "imageGenerationConfig": {
        "numberOfImages": 1,
        "quality": "standard",
        "height": 1024,
        "width": 1024,
        "cfgScale": 8.0,
        "seed": 0
    }
});

let input_string = input.to_string();
let input_bytes = input_string.into_bytes();
let input_blob = Blob::new(input_bytes);

let resp = client.invoke_model()
    .model_id("amazon.titan-image-generator-v1")
    .body(input_blob)
    .send()
    .await?;

    let output_blob: Blob = resp.body; // Assuming resp.body is a Blob
    let output_bytes: &[u8] = output_blob.as_ref(); // Convert Blob to &[u8]
    let output_str = std::str::from_utf8(output_bytes)?; // Convert &[u8] to &str
    let output_string: String = output_str.to_owned(); // Convert &str to String

    let output_json: serde_json::Value = serde_json::from_str(&output_string)?;
    let output_image = output_json["images"][0]["image"].as_str().unwrap();
    let output_image_bytes = decode(output_image)?;
    Ok(output_image_bytes)

}