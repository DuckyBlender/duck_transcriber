use teloxide::{prelude::Requester, types::Message, Bot};

pub fn split_string(input: &str, max_length: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_chunk = String::new();
    let mut current_length = 0;

    for word in input.split_whitespace() {
        if current_length + word.len() + 1 > max_length && !current_chunk.is_empty() {
            result.push(current_chunk);
            current_chunk = String::new();
            current_length = 0;
        }

        if current_length > 0 {
            current_chunk.push(' ');
            current_length += 1;
        }

        current_chunk.push_str(word);
        current_length += word.len();
    }

    if !current_chunk.is_empty() {
        result.push(current_chunk);
    }

    result
}
