use std::env;

use aws_sdk_kms::{primitives::Blob, Client, Error};

pub async fn decrypt_blob(client: &Client, blob: Blob) -> Result<String, Error> {
    let resp = client
        .decrypt()
        .key_id(env::var("KMS_KEY_ID").expect("KMS_KEY_ID not found"))
        .ciphertext_blob(blob)
        .send()
        .await?;

    let inner = resp.plaintext.unwrap();
    let bytes = inner.as_ref();

    let s = String::from_utf8(bytes.to_vec()).expect("Could not convert to UTF-8");

    Ok(s)
}

pub async fn encrypt_string(client: &Client, text: &str) -> Result<Blob, Error> {
    let blob = Blob::new(text.as_bytes());

    let resp = client
        .encrypt()
        .key_id(env::var("KMS_KEY_ID").expect("KMS_KEY_ID not found"))
        .plaintext(blob)
        .send()
        .await?;

    let blob = resp.ciphertext_blob.expect("Could not get encrypted text");

    Ok(blob)
}
