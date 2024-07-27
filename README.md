**duck_transcriber**
=====================================

This is a serverless Telegram bot that transcribes voice, audio, and video notes sent to it using the Groq Whisper API.

**How it works**
---------------

1. The bot receives a voice, audio, or video note message from a user.
2. It downloads the file from Telegram and checks its duration. If the duration is above x minutes, it sends a warning message to the user and exits.
3. It transcribes the audio using the Groq Whisper API.
4. It sends the transcription back to the user as a text message.

**Technical Details**
-------------------

* The bot is built using the `teloxide` crate for interacting with the Telegram API.
* The transcription is done using the `reqwest` crate to send a request to the Groq Whisper API.
* The bot is deployed as a serverless function using AWS Lambda.

**Environment Variables**
-------------------------

* `TELEGRAM_BOT_TOKEN`: the token for the Telegram bot.
* `GROQ_API_KEY`: the API key for the Groq Whisper API.

**Deployment**
------------

Before deploying this bot, ensure you have the following prerequisites installed:

* **AWS CLI**: Follow the instructions [here](https://aws.amazon.com/cli/) to install the AWS Command Line Interface.
* **cargo-lambda**: Install `cargo-lambda` (not with cargo, it doesn't support cross compilation) by following the instructions [here](https://www.cargo-lambda.info/guide/getting-started.html)

To deploy this bot, you'll need to set up an AWS Lambda function and configure it to run this code. You'll also need to set up a Telegram bot and obtain a bot token.

To build:

```bash
cargo lambda build --release --arm64
```

To deploy:

```bash
cargo lambda deploy
```

**License**
-------

Do literally whatever you want with this code. I don't care.

**Contributing**
------------

Contributions are welcome! If you'd like to help improve this bot, please open a pull request with your changes.
