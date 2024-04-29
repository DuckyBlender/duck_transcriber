# Rust AWS Lambda Telegram Bot

## Description

Serverless telegram bot written in Rust which converts audio to text.

## Todo

- [x] Complete basic bot
- [x] Rewrite to use teloxide crate
- [x] /tts command
- [x] /english command
- [x] /help command
- [ ] Keep track of usernames and user IDs (to mention users and check their stats). Such a shame that the Telegram API doesn't provide a way to get the user ID from a mention...
- [ ] Refactor the `dynamodb.rs` file to use methods instead of PartiQL queries
- [ ] Fix command recognition (`text text text /stats text` is being recognized as a command)
- [ ] Remove hard limit (30 minutes) and implement a payment system (don't worry, it will be very cheap 👍)
- [ ] Ban command (admin only)
- [ ] Edit command (admin only)

## Instructions

To run this, install Rust, AWS CLI, AWS SAM CLI and Cargo Lambda
Set the environment variables in the `template-DEFAULT.yaml` file and rename it to `template.yaml`. This is going to be better in the future, I promise.
Then run`sam build` and `sam deploy --guided`

## Contributing

If you think you can make this bot better, just make a pull request. I'll check it out.

## License

Literally do whatever you want I don't care just don't blame me if it doesn't work. This is a project to learn about Rust Lambdas and Webhooks.
