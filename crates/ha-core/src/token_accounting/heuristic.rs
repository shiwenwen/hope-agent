use serde_json::Value;

use super::TokenCountUnknown;

const IMAGE_TOKENS: u64 = 2_000;
const DOCUMENT_TOKENS: u64 = 8_000;
const AUDIO_TOKENS: u64 = 8_000;

pub(crate) fn count_text(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }

    let mut ascii_word_chars = 0u64;
    let mut whitespace = 0u64;
    let mut cjk = 0u64;
    let mut emoji_or_non_bmp = 0u64;
    let mut punctuation = 0u64;
    let mut other = 0u64;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            ascii_word_chars += 1;
        } else if ch.is_ascii_whitespace() {
            whitespace += 1;
        } else if is_cjk(ch) {
            cjk += 1;
        } else if (ch as u32) > 0xffff || is_variation_selector(ch) {
            emoji_or_non_bmp += 1;
        } else if ch.is_ascii_punctuation() {
            punctuation += 1;
        } else {
            other += 1;
        }
    }

    ascii_word_chars
        .div_ceil(4)
        .saturating_add(whitespace.div_ceil(8))
        .saturating_add(cjk)
        .saturating_add(emoji_or_non_bmp.saturating_mul(2))
        .saturating_add(punctuation.div_ceil(2))
        .saturating_add(other)
        .max(1)
}

pub(crate) fn count_json(value: &Value, unknowns: &mut Vec<TokenCountUnknown>) -> u64 {
    match value {
        Value::String(value) if string_contains_inline_image(value) => {
            unknowns.push(TokenCountUnknown::Image);
            IMAGE_TOKENS
        }
        Value::String(value) => count_text(value).saturating_add(2),
        Value::Array(values) => values
            .iter()
            .map(|value| count_json(value, unknowns))
            .sum::<u64>()
            .saturating_add(values.len() as u64)
            .saturating_add(1),
        Value::Object(object) => {
            let media = media_tokens(object, unknowns);
            if media > 0 {
                return media;
            }
            object
                .iter()
                .map(|(key, value)| {
                    count_text(key)
                        .saturating_add(count_json(value, unknowns))
                        .saturating_add(2)
                })
                .sum::<u64>()
                .saturating_add(1)
        }
        Value::Number(value) => count_text(&value.to_string()),
        Value::Bool(_) | Value::Null => 1,
    }
}

pub(crate) fn contains_media(value: &Value) -> bool {
    match value {
        Value::String(value) => string_contains_inline_image(value),
        Value::Array(values) => values.iter().any(contains_media),
        Value::Object(object) => {
            matches!(
                object.get("type").and_then(Value::as_str),
                Some(
                    "image"
                        | "image_url"
                        | "input_image"
                        | "document"
                        | "input_file"
                        | "file"
                        | "input_audio"
                        | "audio"
                )
            ) || object.values().any(contains_media)
        }
        _ => false,
    }
}

fn string_contains_inline_image(value: &str) -> bool {
    value.contains("__IMAGE_BASE64__")
        || value.contains("__IMAGE_FILE__")
        || value.contains("data:image/")
}

fn media_tokens(
    object: &serde_json::Map<String, Value>,
    unknowns: &mut Vec<TokenCountUnknown>,
) -> u64 {
    match object.get("type").and_then(Value::as_str) {
        Some("image") | Some("image_url") | Some("input_image") => {
            unknowns.push(TokenCountUnknown::Image);
            IMAGE_TOKENS
        }
        Some("document") | Some("input_file") | Some("file") => {
            unknowns.push(TokenCountUnknown::Document);
            DOCUMENT_TOKENS
        }
        Some("input_audio") | Some("audio") => {
            unknowns.push(TokenCountUnknown::Audio);
            AUDIO_TOKENS
        }
        _ => 0,
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff |
        0x3040..=0x30ff | 0xac00..=0xd7af)
}

fn is_variation_selector(ch: char) -> bool {
    matches!(ch as u32, 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_does_not_collapse_to_utf8_bytes_div_four() {
        assert_eq!(count_text("你好世界"), 4);
        assert!(count_text("hello world") < 6);
    }

    #[test]
    fn media_is_bounded_without_counting_base64_bytes_as_text() {
        let mut unknowns = Vec::new();
        let value = serde_json::json!({
            "type": "image_url",
            "image_url": {"url": "data:image/png;base64,AAAA"}
        });
        assert_eq!(count_json(&value, &mut unknowns), IMAGE_TOKENS);
        assert_eq!(unknowns, vec![TokenCountUnknown::Image]);
    }

    #[test]
    fn legacy_inline_image_marker_is_not_counted_as_text() {
        let mut unknowns = Vec::new();
        let value = Value::String(format!(
            "__IMAGE_BASE64__image/png__{}__",
            "A".repeat(100_000)
        ));

        assert_eq!(count_json(&value, &mut unknowns), IMAGE_TOKENS);
        assert_eq!(unknowns, vec![TokenCountUnknown::Image]);
    }
}
