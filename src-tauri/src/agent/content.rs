use serde_json::json;

use crate::file_extract;
use super::types::Attachment;

/// Process non-image attachments: extract text and images from files (PDF, Word, Excel, PPT, text).
/// Returns (extra_text to append to message, extra_images as base64 tuples).
pub(super) fn process_file_attachments(attachments: &[Attachment]) -> (String, Vec<file_extract::ExtractedImage>) {
    let mut file_texts = Vec::new();
    let mut extra_images = Vec::new();

    for att in attachments {
        if att.mime_type.starts_with("image/") {
            continue; // Images are handled as multimodal content blocks
        }
        let file_path = match &att.file_path {
            Some(p) => p.as_str(),
            None => continue,
        };

        let content = file_extract::extract(file_path, &att.name, &att.mime_type);

        // Build <file> XML block with path (always present)
        let text_block = match &content.text {
            Some(text) => format!(
                "<file name=\"{}\" path=\"{}\">\n{}\n</file>",
                content.file_name, content.file_path, text
            ),
            None => format!(
                "<file name=\"{}\" path=\"{}\">\n[Binary file. Use tools to inspect if needed.]\n</file>",
                content.file_name, content.file_path
            ),
        };
        file_texts.push(text_block);

        // Collect extracted images (PDF pages, PPT media, etc.)
        extra_images.extend(content.images);
    }

    let extra_text = if file_texts.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", file_texts.join("\n\n"))
    };

    (extra_text, extra_images)
}

/// Build multimodal user content array for Anthropic Messages API.
pub(super) fn build_user_content_anthropic(message: &str, attachments: &[Attachment]) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images) = process_file_attachments(attachments);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    // Check if we have any images (original image attachments + extracted images)
    let has_images = attachments.iter().any(|a| a.mime_type.starts_with("image/"))
        || !extra_images.is_empty();

    if !has_images {
        return json!(full_message);
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();

    // Original image attachments
    for att in attachments {
        if att.mime_type.starts_with("image/") {
            match att.get_base64_data() {
                Ok(b64) => {
                    parts.push(json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": att.mime_type,
                            "data": b64,
                        }
                    }));
                }
                Err(e) => {
                    app_warn!("agent", "attachment", "Skipping attachment {}: {}", att.name, e);
                }
            }
        }
    }

    // Extracted images (PDF pages, PPT media, etc.)
    for img in &extra_images {
        parts.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": img.mime_type,
                "data": img.data,
            }
        }));
    }

    parts.push(json!({ "type": "text", "text": full_message }));
    json!(parts)
}

/// Build multimodal user content array for OpenAI Chat Completions API.
pub(super) fn build_user_content_openai_chat(message: &str, attachments: &[Attachment]) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images) = process_file_attachments(attachments);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    let has_images = attachments.iter().any(|a| a.mime_type.starts_with("image/"))
        || !extra_images.is_empty();

    if !has_images {
        return json!(full_message);
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();

    for att in attachments {
        if att.mime_type.starts_with("image/") {
            match att.get_base64_data() {
                Ok(b64) => {
                    let data_url = format!("data:{};base64,{}", att.mime_type, b64);
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": data_url }
                    }));
                }
                Err(e) => {
                    app_warn!("agent", "attachment", "Skipping attachment {}: {}", att.name, e);
                }
            }
        }
    }

    for img in &extra_images {
        let data_url = format!("data:{};base64,{}", img.mime_type, img.data);
        parts.push(json!({
            "type": "image_url",
            "image_url": { "url": data_url }
        }));
    }

    parts.push(json!({ "type": "text", "text": full_message }));
    json!(parts)
}

/// Build multimodal user content array for OpenAI Responses API / Codex.
pub(super) fn build_user_content_responses(message: &str, attachments: &[Attachment]) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images) = process_file_attachments(attachments);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    let has_images = attachments.iter().any(|a| a.mime_type.starts_with("image/"))
        || !extra_images.is_empty();

    if !has_images {
        return json!(full_message);
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();

    for att in attachments {
        if att.mime_type.starts_with("image/") {
            match att.get_base64_data() {
                Ok(b64) => {
                    let data_url = format!("data:{};base64,{}", att.mime_type, b64);
                    parts.push(json!({
                        "type": "input_image",
                        "image_url": data_url,
                    }));
                }
                Err(e) => {
                    app_warn!("agent", "attachment", "Skipping attachment {}: {}", att.name, e);
                }
            }
        }
    }

    for img in &extra_images {
        let data_url = format!("data:{};base64,{}", img.mime_type, img.data);
        parts.push(json!({
            "type": "input_image",
            "image_url": data_url,
        }));
    }

    parts.push(json!({ "type": "input_text", "text": full_message }));
    json!(parts)
}
