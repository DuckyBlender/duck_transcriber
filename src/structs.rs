use serde::Deserialize;
  
#[derive(Debug, Deserialize)]
pub struct OpenAIResponse {
    pub task: String,
    pub language: String,
    pub duration: f64,
    pub text: String,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Deserialize)]
pub struct Segment {
    pub id: u32,
    pub seek: u32,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub tokens: Vec<u32>,
    pub temperature: f64,
    pub avg_logprob: f64,
    pub compression_ratio: f64,
    pub no_speech_prob: f64,
}