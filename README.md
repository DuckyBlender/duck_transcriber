# Rust AWS Lambda Telegram Bot

## Description

Serverless telegram bot written in Rust which converts audio to text.

## Todo
- [x] Complete basic bot
- [x] Rewrite to use teloxide crate
- [ ] Refactor bot to not send manually requests, but to respond to webhook with a request. Here is an untested example.
```json
{
  "method": "sendMessage",
  "chat_id": <chat_id>,
  "text": "You just sent a message!"
}
```
- [x] /tts command
- [x] /english command

## Instructions

To get a development environment running, clone the repository and navigate into the directory:

```bash
git clone https://github.com/DuckyBlender/duck_transcriber
cd duck_transcriber
```

Then, install cargo lambda. Here is more info
https://www.cargo-lambda.info/

Now that cargo lambda is installed, build the project (preferrably for ARM)

```bash
# some dependencies (fedora)
sudo dnf install perl-core openssl openssl-devel
```

```bash
cargo lambda build --release --arm64
```

Don't forget to set the .env before deploying using
```bash
cargo lambda deploy
```

If you just want to test just run
```bash
cargo lambda watch
```

## Contributing

If you think you can make this bot better, just make a pull request. I'll check it out.

## License

Literally do whatever you want I don't care just don't blame me if it doesn't work. This is a quick project to learn about Rust Lambdas and Webhooks.
