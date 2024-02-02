use crate::dynamodb::insert_data;
use crate::dynamodb::stats;
use crate::openai::{self, TranscribeType, Voice};
use crate::openai::{transcribe_audio, tts};
use crate::utils;
use lambda_http::{Body, Error, Request, Response};
use mime::Mime;
use std::env;
use std::fmt::{Debug, Display, Formatter};
use teloxide::payloads::SendVoiceSetters;
use teloxide::types::ChatAction::{RecordVoice, Typing};
use teloxide::types::MessageEntityKind::BotCommand;
use teloxide::types::{InputFile, ParseMode};
use teloxide::{
    net::Download, payloads::SendMessageSetters, requests::Requester, types::UpdateKind, Bot,
};
use tracing::{error, info};

