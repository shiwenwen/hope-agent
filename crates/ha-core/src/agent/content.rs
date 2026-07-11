use serde_json::json;

use super::types::{Attachment, ProviderFormat};
use crate::file_extract;

/// Process non-image attachments: extract text and images from files (PDF, Word, Excel, PPT, text).
/// Returns (extra_text to append to message, extra_images as base64 tuples).
pub(super) fn process_file_attachments(
    attachments: &[Attachment],
) -> (String, Vec<file_extract::ExtractedImage>) {
    let mut file_texts = Vec::new();
    let mut extra_images = Vec::new();

    for att in attachments {
        // Selected conversation text is carried inline and deliberately uses
        // its own envelope. Treating this as a file quote would invent a path
        // and give the model the wrong provenance.
        if att.source.as_deref() == Some(crate::attachments::MESSAGE_QUOTE_SOURCE) {
            let role = match att.quote_role.as_deref() {
                Some("user") => "user",
                Some("assistant") => "assistant",
                _ => "unknown",
            };
            let snippet = att.data.as_deref().unwrap_or("");
            file_texts.push(format!(
                "<message_quote role=\"{}\">\n{}\n</message_quote>",
                role,
                escape_xml_text(snippet)
            ));
            continue;
        }
        // File-browser "quote to chat": emit a <file_reference> block from the
        // carried snippet instead of reading a file. The user only ever sees a
        // friendly quote card; the model sees this structured reference.
        if att.source.as_deref() == Some("quote") {
            let path = att.file_path.as_deref().unwrap_or(att.name.as_str());
            let snippet = att.data.as_deref().unwrap_or("");
            let lines_attr = match att.quote_lines.as_deref().filter(|s| !s.is_empty()) {
                Some(lines) => format!(" lines=\"{}\"", escape_xml_attr(lines)),
                None => String::new(),
            };
            // Escape the attribute and the body so a snippet (or path)
            // containing markup — e.g. a literal `</file_reference>` — cannot
            // break out of the reference and inject text the model would read
            // as its own instructions.
            file_texts.push(format!(
                "<file_reference path=\"{}\"{}>\n{}\n</file_reference>",
                escape_xml_attr(path),
                lines_attr,
                escape_xml_text(snippet)
            ));
            continue;
        }
        if att.source.as_deref() == Some(crate::attachments::PASTED_TEXT_SOURCE) {
            let Some(file_path) = att.file_path.as_deref() else {
                continue;
            };
            let size_attr = std::fs::metadata(file_path)
                .map(|m| format!(" size_bytes=\"{}\"", m.len()))
                .unwrap_or_default();
            file_texts.push(format!(
                "<file name=\"{}\" path=\"{}\" source=\"{}\"{}>\n[Pasted text was saved as a session attachment. Use the read tool with this path to inspect the full content when needed.]\n</file>",
                escape_xml_attr(&att.name),
                escape_xml_attr(file_path),
                crate::attachments::PASTED_TEXT_SOURCE,
                size_attr
            ));
            continue;
        }
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

/// Escape a value for use inside a double-quoted XML attribute.
fn escape_xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

/// Minimal XML text escaping: neutralizing `<` (and `&`) is enough to stop any
/// embedded markup — e.g. a literal `</file_reference>` — from closing the
/// surrounding element. `>` is left intact so the quoted source stays readable.
fn escape_xml_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
}

/// Build multimodal user content array for Anthropic Messages API.
pub(super) fn build_user_content_anthropic(
    message: &str,
    attachments: &[Attachment],
) -> serde_json::Value {
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
    let has_images = attachments
        .iter()
        .any(|a| a.mime_type.starts_with("image/"))
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
                    app_warn!(
                        "agent",
                        "attachment",
                        "Skipping attachment {}: {}",
                        att.name,
                        e
                    );
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
pub(super) fn build_user_content_openai_chat(
    message: &str,
    attachments: &[Attachment],
) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images) = process_file_attachments(attachments);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    let has_images = attachments
        .iter()
        .any(|a| a.mime_type.starts_with("image/"))
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
                    app_warn!(
                        "agent",
                        "attachment",
                        "Skipping attachment {}: {}",
                        att.name,
                        e
                    );
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
pub(super) fn build_user_content_responses(
    message: &str,
    attachments: &[Attachment],
) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images) = process_file_attachments(attachments);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    let has_images = attachments
        .iter()
        .any(|a| a.mime_type.starts_with("image/"))
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
                    app_warn!(
                        "agent",
                        "attachment",
                        "Skipping attachment {}: {}",
                        att.name,
                        e
                    );
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

pub(super) fn build_user_content_for_provider(
    provider_format: ProviderFormat,
    message: &str,
    attachments: &[Attachment],
) -> serde_json::Value {
    match provider_format {
        ProviderFormat::Anthropic => build_user_content_anthropic(message, attachments),
        ProviderFormat::OpenAIChat => build_user_content_openai_chat(message, attachments),
        ProviderFormat::OpenAIResponses | ProviderFormat::Codex => {
            build_user_content_responses(message, attachments)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_text_attachment_injects_reference_without_body() {
        let attachments = vec![Attachment {
            name: "pasted <text>.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some(crate::attachments::PASTED_TEXT_SOURCE.to_string()),
            data: Some("this body should stay on disk".to_string()),
            file_path: Some("/tmp/paste <secret>.txt".to_string()),
            quote_lines: None,
            quote_role: None,
        }];

        let (text, images) = process_file_attachments(&attachments);

        assert!(images.is_empty());
        assert!(text.contains("source=\"pasted_text\""));
        assert!(text.contains("Use the read tool"));
        assert!(text.contains("pasted &lt;text>.txt"));
        assert!(text.contains("/tmp/paste &lt;secret>.txt"));
        assert!(!text.contains("this body should stay on disk"));
    }

    #[test]
    fn message_quote_uses_its_own_escaped_envelope() {
        let attachments = vec![Attachment {
            name: "message-quote".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some(crate::attachments::MESSAGE_QUOTE_SOURCE.to_string()),
            data: Some("Keep <this> & never </message_quote>".to_string()),
            file_path: None,
            quote_lines: None,
            quote_role: Some("assistant".to_string()),
        }];

        let (text, images) = process_file_attachments(&attachments);

        assert!(images.is_empty());
        assert!(text.contains("<message_quote role=\"assistant\">"));
        assert!(text.contains("Keep &lt;this> &amp; never &lt;/message_quote>"));
        assert_eq!(text.matches("</message_quote>").count(), 1);
        assert!(!text.contains("<file_reference"));
    }
}
