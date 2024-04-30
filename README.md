# Rust AWS Lambda Telegram Bot

## Description

Serverless telegram bot written in Rust which converts audio to text.

## Todo

- [x] Complete basic bot
- [x] Rewrite to use teloxide crate
- [x] /tts command
- [x] /english command
- [x] /help command
- [ ] Use AWS SAM to prepare for more changes (**THIS FORK IS STILL WIP**)
- [ ] Add the permission to access the DynamoDB table to the template.yaml (if that's even possible)
- [ ] Keep track of usernames and user IDs (to mention users and check their stats). Such a shame that the Telegram API doesn't provide a way to get the user ID from a mention...
- [ ] Refactor the `dynamodb.rs` file to use methods instead of PartiQL queries
- [ ] Fix command recognition (`text text text /stats text` is being recognized as a command)
- [ ] Remove hard limit (30 minutes) and implement a payment system (don't worry, it will be very cheap 👍)
- [ ] Ban command (admin only)
- [ ] Edit command (admin only)

## Instructions

To run this, install Rust, AWS CLI, AWS SAM CLI and Cargo Lambda
Set the SecretArn SecretsManager variable in the `template.yaml`. You'll need to create a secret with the following keys:
`OPENAI_API_KEY`, `TELEGRAM_BOT_TOKEN` and `TELEGRAM_OWNER_ID`
Then run `sam build` and `sam deploy --guided`
Then set the telegram bot webhook URL to the API gateway (will be printed in the console)
THEN give permission to the lambda to access the DynamoDB table (todo: add this to the template.yaml)

## Contributing

If you think you can make this bot better, just make a pull request. I'll check it out.

## License

Literally do whatever you want I don't care just don't blame me if it doesn't work. This is a project to learn about Rust Lambdas and Webhooks.
