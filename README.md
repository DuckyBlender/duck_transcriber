# Duck Transcriber

Duck Transcriber is a bot which transcribes voice messages to text.

## Chat Settings Schema

The `transcriber` table in the database has the following structure:

| Column Name | Data Type | Key | Default Value | Description |
|-------------|-----------|-----|---------------|-------------|
| chat_id     | BIGINT    | PRIMARY KEY | None | Unique identifier for the chat |
| debug_info  | BOOLEAN   |     | false | Enables or disables debug information |
| delete_voice| BOOLEAN   |     | false | Option to delete the original voice message after transcription |
| gpt_enhance | BOOLEAN   |     | false | Enhances transcribed text with GPT |