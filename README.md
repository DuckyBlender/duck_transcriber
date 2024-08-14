# **duck_transcriber**

## Overview

This is a serverless Telegram bot that transcribes voice, audio, and video notes sent to it using the Groq Whisper API. It also stores and retrieves transcriptions using AWS DynamoDB, with a focus on user privacy. All transcriptions are encrypted using AWS Key Management Service (KMS) to ensure that only the bot can access the transcribed text.

## **How It Works**

1. The bot receives a voice, audio, or video note message from a user.
2. It downloads the file from Telegram and checks its duration. If the duration is above a specified limit, it sends a warning message to the user and exits.
3. The bot checks if the transcription already exists in AWS DynamoDB. If it does, the saved transcription is decrypted using AWS KMS and sent back to the user.
4. If no transcription is found, the bot transcribes the audio using the Groq Whisper API.
5. The transcription is sent back to the user as a text message, encrypted using AWS KMS, and stored in DynamoDB for future reference.

## **Supported Commands**

- `/start`: Initializes the bot and provides a welcome message.
- `/help`: Provides information on how to use the bot and its features.
- more coming soon!

## **Technical Details**

- The bot is built using the `teloxide` crate for interacting with the Telegram API.
- The transcription is done using the `reqwest` crate to send a request to the Groq Whisper API.
- The bot uses AWS DynamoDB to store and retrieve transcriptions, ensuring that repeated requests for the same audio do not require retranscription.
- The bot uses AWS Key Management Service (KMS) to encrypt and decrypt transcriptions, ensuring that only the bot can access the transcribed text.
- The bot is deployed as a serverless function using AWS Lambda.

## **Environment Variables**

- `TELEGRAM_BOT_TOKEN`: the token for the Telegram bot.
- `GROQ_API_KEY`: the API key for the Groq Whisper API.
- `DYNAMODB_TABLE`: the name of the DynamoDB table where transcriptions are stored.
- `KMS_KEY_ID`: the ID of the AWS KMS key used for encryption and decryption.

## **Deployment**

Before deploying this bot, ensure you have the following prerequisites installed:

- **AWS CLI**: Follow the instructions [here](https://aws.amazon.com/cli/) to install the AWS Command Line Interface.
- **cargo-lambda**: Install `cargo-lambda` (not with cargo, it doesn't support cross compilation) by following the instructions [here](https://www.cargo-lambda.info/guide/getting-started.html).

To build:

```bash
cargo lambda build --release --arm64
```

To deploy:

```bash
cargo lambda deploy
```

### **AWS Lambda Permissions**

Ensure that your AWS Lambda function has the necessary permissions to access DynamoDB and AWS KMS. You will need to attach a policy that grants the Lambda function read and write permissions to the DynamoDB table, as well as permissions to use the AWS KMS key for encryption and decryption. This can be done by attaching the `AWSLambdaDynamoDBExecutionRole` managed policy or by creating a custom policy with the necessary permissions.

## **License**

Do literally whatever you want with this code. I don't care.

## **Contributing**

Contributions are welcome! If you'd like to help improve this bot, please open a pull request with your changes.
