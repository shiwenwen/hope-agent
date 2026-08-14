use anyhow::Context;
use serde_json::json;

fn warn_attachment_read_failure(error: &impl std::fmt::Display) {
    let failure = crate::cache_routing::audit_fingerprint(
        "attachment-provider-read",
        error.to_string().as_bytes(),
    );
    app_warn!(
        "agent",
        "attachment",
        "Skipping one unavailable attachment ({})",
        &failure[..16]
    );
}

fn is_frozen_context_resource(source: Option<&str>) -> bool {
    matches!(source, Some("mention" | "plan_mention"))
}

use base64::Engine;

use super::types::{Attachment, ProviderFormat};
use crate::file_extract;

/// Process non-image attachments: extract text and images from files (PDF, Word, Excel, PPT, text).
/// Returns (extra_text to append to message, extra_images as base64 tuples).
pub(super) fn process_file_attachments(
    attachments: &[Attachment],
    context_window: u32,
    context_resource_refs: &[crate::prompt_context::ContextResourceRef],
) -> (String, Vec<file_extract::ExtractedImage>, bool) {
    let mut file_texts = Vec::new();
    let mut extra_images = Vec::new();
    let has_typed_resources = attachments
        .iter()
        .any(|attachment| is_frozen_context_resource(attachment.source.as_deref()));
    let mut typed_delivery_allowed = true;
    let typed_extraction_baseline =
        match crate::attachments::typed_resource_extraction_budget_bytes(attachments) {
            Ok(remaining) => remaining,
            Err(error) => {
                warn_attachment_read_failure(&error);
                typed_delivery_allowed = false;
                None
            }
        };
    if has_typed_resources && typed_extraction_baseline.is_none() {
        typed_delivery_allowed = false;
    }
    let mut typed_ledger = None;
    let mut typed_extraction_start_remaining = typed_extraction_baseline;
    if let Some(baseline) = typed_extraction_baseline {
        match lock_initial_context_resource_budget(context_resource_refs, baseline) {
            Ok((ledger, available)) => {
                typed_ledger = Some(ledger);
                typed_extraction_start_remaining = Some(available);
            }
            Err(error) => {
                warn_attachment_read_failure(&error);
                typed_delivery_allowed = false;
                typed_extraction_start_remaining = None;
            }
        }
    }
    let mut typed_extraction_budget =
        typed_extraction_start_remaining.map(file_extract::FrozenExtractionBudget::new);
    let materialized_resource_count = attachments
        .iter()
        .filter(|attachment| {
            attachment.source.as_deref() != Some(crate::attachments::MESSAGE_QUOTE_SOURCE)
                && attachment.source.as_deref() != Some("quote")
                && attachment.source.as_deref() != Some(crate::attachments::PASTED_TEXT_SOURCE)
                && !attachment.mime_type.starts_with("image/")
                && attachment.file_path.is_some()
                && (typed_delivery_allowed
                    || !is_frozen_context_resource(attachment.source.as_deref()))
        })
        .count();
    let per_resource_text_chars = file_extract::resource_materialization_budget_chars(
        context_window,
        materialized_resource_count,
    );

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
            file_texts.push((
                false,
                format!(
                    "<message_quote role=\"{}\">\n{}\n</message_quote>",
                    role,
                    escape_xml_text(snippet)
                ),
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
            file_texts.push((
                false,
                format!(
                    "<file_reference path=\"{}\"{}>\n{}\n</file_reference>",
                    escape_xml_attr(path),
                    lines_attr,
                    escape_xml_text(snippet)
                ),
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
            file_texts.push((false, format!(
                "<file name=\"{}\" path=\"{}\" source=\"{}\"{}>\n[Pasted text was saved as a session attachment. Use the read tool with this path to inspect the full content when needed.]\n</file>",
                escape_xml_attr(&att.name),
                escape_xml_attr(file_path),
                crate::attachments::PASTED_TEXT_SOURCE,
                size_attr
            )));
            continue;
        }
        if att.mime_type.starts_with("image/") {
            continue; // Images are handled as multimodal content blocks
        }
        let file_path = match &att.file_path {
            Some(p) => p.as_str(),
            None => continue,
        };
        let provider_path = if is_frozen_context_resource(att.source.as_deref()) {
            att.name.as_str()
        } else {
            file_path
        };

        // Incognito typed mentions carry their source bytes in-memory. Extract
        // from those frozen bytes with the bounded in-memory parser so provider
        // failover never re-reads a mutable source file. Normal sessions also
        // retain a durable managed snapshot, but the live attempt consumes the
        // in-memory frozen bytes here.
        let is_frozen = is_frozen_context_resource(att.source.as_deref());
        let (mut content, mut reference_only, typed_text_needs_charge) = if is_frozen {
            match att.data.as_deref() {
                Some(data) => match base64::engine::general_purpose::STANDARD.decode(data) {
                    Ok(bytes) if file_extract::is_text_like(&att.mime_type, &att.name) => {
                        match String::from_utf8(bytes) {
                            Ok(text) => (
                                file_extract::FileContent {
                                    text: Some(text),
                                    images: Vec::new(),
                                    file_path: att.name.clone(),
                                    file_name: att.name.clone(),
                                    extraction_truncated: false,
                                },
                                false,
                                true,
                            ),
                            Err(error) => {
                                warn_attachment_read_failure(&error);
                                (frozen_reference_content(att), true, false)
                            }
                        }
                    }
                    Ok(bytes) => match typed_extraction_budget.as_mut() {
                        Some(budget) => {
                            let mut trial = budget.clone();
                            match file_extract::extract_frozen_bytes_bounded(
                                &bytes,
                                &att.name,
                                &att.mime_type,
                                &mut trial,
                                true,
                                per_resource_text_chars,
                            ) {
                                Ok(content) => {
                                    *budget = trial;
                                    (content, false, false)
                                }
                                Err(error) => {
                                    warn_attachment_read_failure(&error);
                                    (frozen_reference_content(att), true, false)
                                }
                            }
                        }
                        None => (frozen_reference_content(att), true, false),
                    },
                    Err(error) => {
                        warn_attachment_read_failure(&error);
                        (frozen_reference_content(att), true, false)
                    }
                },
                // Reopening a compatibility path here would leave the bounded
                // frozen-resource model. Keep it as an explicit reference.
                None => (frozen_reference_content(att), true, false),
            }
        } else {
            (
                file_extract::extract(file_path, &att.name, &att.mime_type),
                false,
                false,
            )
        };

        if typed_text_needs_charge {
            let projected_text_bytes = content
                .text
                .as_deref()
                .map(|text| projected_materialized_text_bytes(text, per_resource_text_chars))
                .unwrap_or_default();
            let charge = typed_extraction_budget
                .as_mut()
                .context("typed extraction budget is unavailable")
                .and_then(|budget| budget.retain_provider_text(projected_text_bytes));
            if let Err(error) = charge {
                warn_attachment_read_failure(&error);
                content = frozen_reference_content(att);
                reference_only = true;
            }
        }

        // Build <file> XML block with path (always present)
        let text_block = if reference_only {
            format!(
                "<file name=\"{}\" path=\"{}\" materialization=\"reference\" continuation_tool=\"{}\">\n[Automatic extraction was deferred to preserve the typed-resource memory ceiling. Use the continuation tool with this turn's opaque resourceRef; unsupported compressed formats remain available through bounded Base64 paging.]\n</file>",
                escape_xml_attr(&content.file_name),
                escape_xml_attr(provider_path),
                crate::tool_defs::TOOL_READ_CONTEXT_RESOURCE,
            )
        } else {
            match &content.text {
            Some(text) => {
                let materialized = materialize_resource_text(text, per_resource_text_chars);
                let continuation_tool = if is_frozen {
                    crate::tool_defs::TOOL_READ_CONTEXT_RESOURCE
                } else {
                    crate::tool_defs::TOOL_READ
                };
                format!(
                    "<file name=\"{}\" path=\"{}\" materialization=\"{}\" included_chars=\"{}\" extracted_chars=\"{}\" extraction_truncated=\"{}\" continuation_tool=\"{}\">\n{}\n</file>",
                    escape_xml_attr(&content.file_name),
                    escape_xml_attr(provider_path),
                    materialized.kind,
                    materialized.included_chars,
                    materialized.extracted_chars,
                    content.extraction_truncated,
                    continuation_tool,
                    escape_xml_text(&materialized.text)
                )
            }
            None => format!(
                "<file name=\"{}\" path=\"{}\">\n[Binary file. Use tools to inspect if needed.]\n</file>",
                escape_xml_attr(&content.file_name),
                escape_xml_attr(provider_path)
            ),
            }
        };
        file_texts.push((is_frozen, text_block));

        // Collect extracted images (PDF pages, PPT media, etc.)
        extra_images.extend(content.images.into_iter().map(|image| (is_frozen, image)));
    }

    if let (Some(baseline), Some(start), Some(budget), Some(mut ledger_guard)) = (
        typed_extraction_baseline,
        typed_extraction_start_remaining,
        typed_extraction_budget.as_ref(),
        typed_ledger,
    ) {
        let commit = (|| -> anyhow::Result<()> {
            let consumed = start
                .checked_sub(budget.remaining_bytes())
                .context("typed extraction budget accounting underflow")?;
            let ledger = ledger_guard
                .as_mut()
                .context("typed extraction ledger was not initialized")?;
            let initial = ledger.initial_materialization_consumed_bytes.max(consumed);
            let total = initial
                .checked_add(ledger.continuation_consumed_bytes)
                .context("typed extraction budget accounting overflow")?;
            if total > baseline {
                anyhow::bail!("typed extraction materialization exceeds its turn budget");
            }
            // Provider/profile rebuilds materialize the same initial message.
            // Replace conservatively with the largest projection; only
            // continuation tool results accumulate.
            ledger.initial_materialization_consumed_bytes = initial;
            Ok(())
        })();
        if let Err(error) = commit {
            warn_attachment_read_failure(&error);
            typed_delivery_allowed = false;
        }
    }

    if !typed_delivery_allowed {
        file_texts.retain(|(is_typed, _)| !is_typed);
        extra_images.retain(|(is_typed, _)| !is_typed);
        file_texts.extend(
            typed_resource_accounting_failure_references(attachments)
                .into_iter()
                .map(|reference| (true, reference)),
        );
    }

    let extra_text = if file_texts.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n{}",
            file_texts
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join("\n\n")
        )
    };
    let extra_images = extra_images.into_iter().map(|(_, image)| image).collect();
    (extra_text, extra_images, typed_delivery_allowed)
}

type ContextResourceLedgerGuard<'a> =
    std::sync::MutexGuard<'a, Option<crate::prompt_context::ContextResourceBudgetLedger>>;

fn lock_initial_context_resource_budget<'a>(
    resources: &'a [crate::prompt_context::ContextResourceRef],
    baseline_remaining_bytes: usize,
) -> anyhow::Result<(ContextResourceLedgerGuard<'a>, usize)> {
    let first = resources
        .first()
        .context("typed attachments have no turn-scoped context resource refs")?;
    if resources
        .iter()
        .any(|resource| !std::sync::Arc::ptr_eq(&resource.turn_budget, &first.turn_budget))
    {
        anyhow::bail!("context resources do not share one turn-owned budget ledger");
    }
    let rebuilt_baseline = crate::attachments::context_resource_extraction_budget_bytes(resources)?;
    if rebuilt_baseline != baseline_remaining_bytes {
        anyhow::bail!("typed attachment baseline does not match its frozen resource refs");
    }
    let resource_refs = resources
        .iter()
        .map(|resource| resource.resource_ref.clone())
        .collect::<Vec<_>>();
    let mut ledger_guard = first
        .turn_budget
        .ledger
        .lock()
        .map_err(|_| anyhow::anyhow!("context resource turn budget ledger is poisoned"))?;
    let ledger =
        ledger_guard.get_or_insert_with(|| crate::prompt_context::ContextResourceBudgetLedger {
            baseline_remaining_bytes,
            initial_materialization_consumed_bytes: 0,
            continuation_consumed_bytes: 0,
            resource_refs: resource_refs.clone(),
        });
    if ledger.baseline_remaining_bytes != baseline_remaining_bytes
        || ledger.resource_refs != resource_refs
    {
        anyhow::bail!("initial context materialization does not match its turn budget ledger");
    }
    let already_consumed = ledger
        .initial_materialization_consumed_bytes
        .checked_add(ledger.continuation_consumed_bytes)
        .context("context resource turn budget accounting overflow")?;
    if already_consumed > baseline_remaining_bytes {
        anyhow::bail!("context resource turn budget is exhausted");
    }
    // A provider/profile rebuild replaces the initial materialization and may
    // reuse its allowance, but it must preserve every continuation result
    // already retained in canonical/provider history.
    let available_after_continuations = baseline_remaining_bytes
        .checked_sub(ledger.continuation_consumed_bytes)
        .context("context resource turn budget is exhausted")?;
    let protected_continuation = crate::attachments::TYPED_RESOURCE_CONTINUATION_FLOOR_BYTES
        .min(available_after_continuations);
    let available = available_after_continuations
        .checked_sub(protected_continuation)
        .context("context resource continuation floor accounting underflow")?;
    Ok((ledger_guard, available))
}

fn typed_resource_accounting_failure_references(attachments: &[Attachment]) -> Vec<String> {
    attachments
        .iter()
        .filter(|attachment| is_frozen_context_resource(attachment.source.as_deref()))
        .map(|attachment| {
            format!(
                "<file name=\"{}\" materialization=\"reference\">\n[Typed resource delivery was deferred because its turn memory ledger could not be validated.]\n</file>",
                escape_xml_attr(&attachment.name),
            )
        })
        .collect()
}

fn frozen_reference_content(att: &Attachment) -> file_extract::FileContent {
    file_extract::FileContent {
        text: None,
        images: Vec::new(),
        file_path: att.name.clone(),
        file_name: att.name.clone(),
        extraction_truncated: false,
    }
}

fn projected_materialized_text_bytes(text: &str, max_chars: usize) -> usize {
    if text.chars().count() <= max_chars {
        text.len()
    } else {
        max_chars.saturating_mul(4).saturating_add(512)
    }
}

struct MaterializedResourceText {
    text: String,
    kind: &'static str,
    included_chars: usize,
    extracted_chars: usize,
}

/// Keep small resources complete. For a large resource, retain deterministic
/// head and tail evidence instead of silently chopping at an arbitrary byte;
/// the immutable snapshot remains available through the scoped continuation
/// tool advertised alongside the binding.
fn materialize_resource_text(text: &str, max_chars: usize) -> MaterializedResourceText {
    let extracted_chars = text.chars().count();
    if extracted_chars <= max_chars {
        return MaterializedResourceText {
            text: text.to_string(),
            kind: "full",
            included_chars: extracted_chars,
            extracted_chars,
        };
    }

    let notice_reserve = 256usize.min(max_chars / 4);
    let content_budget = max_chars.saturating_sub(notice_reserve).max(1);
    let head_chars = content_budget.saturating_mul(3) / 4;
    let tail_chars = content_budget.saturating_sub(head_chars);
    let head = text.chars().take(head_chars).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    let included_chars = head.chars().count().saturating_add(tail.chars().count());
    MaterializedResourceText {
        text: format!(
            "{head}\n\n[Preview omitted {} extracted characters. Use the advertised continuation tool and the typed binding's opaque resourceRef to read the same frozen version.]\n\n{tail}",
            extracted_chars.saturating_sub(included_chars)
        ),
        kind: "preview",
        included_chars,
        extracted_chars,
    }
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
    context_window: u32,
    context_resource_refs: &[crate::prompt_context::ContextResourceRef],
) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images, typed_delivery_allowed) =
        process_file_attachments(attachments, context_window, context_resource_refs);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    // Check if we have any images (original image attachments + extracted images)
    let has_images = attachments.iter().any(|a| {
        a.mime_type.starts_with("image/")
            && (typed_delivery_allowed || !is_frozen_context_resource(a.source.as_deref()))
    }) || !extra_images.is_empty();

    if !has_images {
        return json!(full_message);
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();

    // Original image attachments
    for att in attachments {
        if att.mime_type.starts_with("image/")
            && (typed_delivery_allowed || !is_frozen_context_resource(att.source.as_deref()))
        {
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
                Err(e) => warn_attachment_read_failure(&e),
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
    context_window: u32,
    context_resource_refs: &[crate::prompt_context::ContextResourceRef],
) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images, typed_delivery_allowed) =
        process_file_attachments(attachments, context_window, context_resource_refs);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    let has_images = attachments.iter().any(|a| {
        a.mime_type.starts_with("image/")
            && (typed_delivery_allowed || !is_frozen_context_resource(a.source.as_deref()))
    }) || !extra_images.is_empty();

    if !has_images {
        return json!(full_message);
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();

    for att in attachments {
        if att.mime_type.starts_with("image/")
            && (typed_delivery_allowed || !is_frozen_context_resource(att.source.as_deref()))
        {
            match att.get_base64_data() {
                Ok(b64) => {
                    let data_url = format!("data:{};base64,{}", att.mime_type, b64);
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": data_url }
                    }));
                }
                Err(e) => warn_attachment_read_failure(&e),
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
    context_window: u32,
    context_resource_refs: &[crate::prompt_context::ContextResourceRef],
) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(message);
    }

    let (extra_text, extra_images, typed_delivery_allowed) =
        process_file_attachments(attachments, context_window, context_resource_refs);
    let full_message = if extra_text.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", message, extra_text)
    };

    let has_images = attachments.iter().any(|a| {
        a.mime_type.starts_with("image/")
            && (typed_delivery_allowed || !is_frozen_context_resource(a.source.as_deref()))
    }) || !extra_images.is_empty();

    if !has_images {
        return json!(full_message);
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();

    for att in attachments {
        if att.mime_type.starts_with("image/")
            && (typed_delivery_allowed || !is_frozen_context_resource(att.source.as_deref()))
        {
            match att.get_base64_data() {
                Ok(b64) => {
                    let data_url = format!("data:{};base64,{}", att.mime_type, b64);
                    parts.push(json!({
                        "type": "input_image",
                        "image_url": data_url,
                    }));
                }
                Err(e) => warn_attachment_read_failure(&e),
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
    context_window: u32,
    context_resource_refs: &[crate::prompt_context::ContextResourceRef],
) -> serde_json::Value {
    match provider_format {
        ProviderFormat::Anthropic => build_user_content_anthropic(
            message,
            attachments,
            context_window,
            context_resource_refs,
        ),
        ProviderFormat::OpenAIChat => build_user_content_openai_chat(
            message,
            attachments,
            context_window,
            context_resource_refs,
        ),
        ProviderFormat::OpenAIResponses | ProviderFormat::Codex => build_user_content_responses(
            message,
            attachments,
            context_window,
            context_resource_refs,
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};
    use std::sync::Arc;

    use super::*;
    use zip::write::SimpleFileOptions;

    fn context_resources(
        attachments: &[Attachment],
    ) -> Vec<crate::prompt_context::ContextResourceRef> {
        let turn_budget = Arc::new(crate::prompt_context::ContextResourceTurnBudget::default());
        attachments
            .iter()
            .enumerate()
            .filter(|(_, attachment)| is_frozen_context_resource(attachment.source.as_deref()))
            .map(
                |(index, attachment)| crate::prompt_context::ContextResourceRef {
                    resource_ref: format!("resource_ref_{index}"),
                    mention_id: format!("mention-{index}"),
                    target_id: attachment.name.clone(),
                    file_name: attachment.name.clone(),
                    mime_type: attachment.mime_type.clone(),
                    parent_session_id: "session-1".into(),
                    parent_turn_id: Some("turn-1".into()),
                    principal_agent_id: "agent-1".into(),
                    bytes: Arc::from(
                        base64::engine::general_purpose::STANDARD
                            .decode(attachment.data.as_deref().expect("frozen Base64"))
                            .expect("valid frozen Base64"),
                    ),
                    turn_budget: turn_budget.clone(),
                },
            )
            .collect()
    }

    #[test]
    fn inline_base64_is_borrowed_until_provider_payload_owns_it() {
        let attachment = Attachment {
            name: "image.png".to_string(),
            mime_type: "image/png".to_string(),
            source: Some("mention".to_string()),
            data: Some("YWJj".to_string()),
            file_path: Some("/frozen/image.png".to_string()),
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_role: None,
            quote_project_root: None,
            quote_worktree_root: None,
        };
        assert!(matches!(
            attachment.get_base64_data().expect("base64"),
            std::borrow::Cow::Borrowed("YWJj")
        ));
        let resources = context_resources(std::slice::from_ref(&attachment));
        let content = build_user_content_openai_chat("inspect", &[attachment], 8_000, &resources);
        assert_eq!(content[0]["image_url"]["url"], "data:image/png;base64,YWJj");
    }

    #[test]
    fn pasted_text_attachment_injects_reference_without_body() {
        let attachments = vec![Attachment {
            name: "pasted <text>.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some(crate::attachments::PASTED_TEXT_SOURCE.to_string()),
            data: Some("this body should stay on disk".to_string()),
            file_path: Some("/tmp/paste <secret>.txt".to_string()),
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_role: None,
            quote_project_root: None,
            quote_worktree_root: None,
        }];

        let (text, images, _) = process_file_attachments(&attachments, 200_000, &[]);

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
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_role: Some("assistant".to_string()),
            quote_project_root: None,
            quote_worktree_root: None,
        }];

        let (text, images, _) = process_file_attachments(&attachments, 200_000, &[]);

        assert!(images.is_empty());
        assert!(text.contains("<message_quote role=\"assistant\">"));
        assert!(text.contains("Keep &lt;this> &amp; never &lt;/message_quote>"));
        assert_eq!(text.matches("</message_quote>").count(), 1);
        assert!(!text.contains("<file_reference"));
    }

    #[test]
    fn typed_plan_extraction_uses_frozen_bytes_not_snapshot_path() {
        let attachments = vec![Attachment {
            name: "plan.md".to_string(),
            mime_type: "text/markdown".to_string(),
            source: Some("plan_mention".to_string()),
            data: Some(base64::engine::general_purpose::STANDARD.encode(b"# Frozen plan")),
            file_path: Some("/path/that/must/not/be/reopened.md".to_string()),
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_role: None,
            quote_project_root: None,
            quote_worktree_root: None,
        }];

        let resources = context_resources(&attachments);
        let (text, images, _) = process_file_attachments(&attachments, 200_000, &resources);

        assert!(images.is_empty());
        assert!(text.contains("# Frozen plan"));
        assert!(!text.contains("/path/that/must/not/be/reopened.md"));
    }

    #[test]
    fn large_resources_are_previewed_with_fair_model_budget() {
        let attachments = (0..2)
            .map(|index| Attachment {
                name: format!("large-{index}.txt"),
                mime_type: "text/plain".to_string(),
                source: Some("mention".to_string()),
                data: Some(base64::engine::general_purpose::STANDARD.encode(format!(
                    "HEAD-{index}\n{}\nTAIL-{index}",
                    "x".repeat(20_000)
                ))),
                file_path: Some(format!("/snapshot-{index}.txt")),
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            })
            .collect::<Vec<_>>();

        let resources = context_resources(&attachments);
        let (text, images, _) = process_file_attachments(&attachments, 8_000, &resources);

        assert!(images.is_empty());
        assert_eq!(text.matches("materialization=\"preview\"").count(), 2);
        assert_eq!(text.matches("Preview omitted").count(), 2);
        assert!(text.contains("HEAD-0"));
        assert!(text.contains("TAIL-1"));
        assert!(text.contains("continuation_tool=\"read_context_resource\""));
        assert!(text.len() < 12_000);
    }

    #[test]
    fn bounded_office_preview_advertises_real_text_continuation() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "word/document.xml",
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .expect("DOCX entry");
        write!(
            writer,
            "<w:document><w:body><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:body></w:document>",
            "office".repeat(4_000)
        )
        .expect("DOCX XML");
        let bytes = writer.finish().expect("DOCX").into_inner();
        let attachment = Attachment {
            name: "large.docx".to_string(),
            mime_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                .to_string(),
            source: Some("mention".to_string()),
            data: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            file_path: Some("/immutable/large.docx".to_string()),
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_role: None,
            quote_project_root: None,
            quote_worktree_root: None,
        };

        let resources = context_resources(std::slice::from_ref(&attachment));
        let (text, images, _) = process_file_attachments(&[attachment], 8_000, &resources);
        assert!(images.is_empty());
        assert!(text.contains("materialization=\"preview\""));
        assert!(text.contains("extraction_truncated=\"false\""));
        assert!(text.contains("continuation_tool=\"read_context_resource\""));
    }

    #[tokio::test]
    async fn pptx_media_initial_charge_is_shared_with_continuation_and_rebuild_is_idempotent() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file("ppt/slides/slide1.xml", options)
            .expect("slide entry");
        writer
            .write_all(b"<p:sld><a:p><a:r><a:t>Ledger</a:t></a:r></a:p></p:sld>")
            .expect("slide XML");
        writer
            .start_file("ppt/media/image1.png", options)
            .expect("media entry");
        writer
            .write_all(&vec![0x5a; 4 * 1024])
            .expect("media bytes");
        let bytes = writer.finish().expect("PPTX").into_inner();
        let attachment = Attachment {
            name: "deck.pptx".into(),
            mime_type: "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                .into(),
            source: Some("mention".into()),
            data: Some(base64::engine::general_purpose::STANDARD.encode(&bytes)),
            file_path: Some("/immutable/deck.pptx".into()),
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_role: None,
            quote_project_root: None,
            quote_worktree_root: None,
        };
        let resources = context_resources(std::slice::from_ref(&attachment));

        let (_, images, allowed) =
            process_file_attachments(std::slice::from_ref(&attachment), 8_000, &resources);
        assert!(allowed);
        assert_eq!(images.len(), 1);
        let initial = {
            let ledger = resources[0].turn_budget.ledger.lock().expect("ledger");
            ledger
                .as_ref()
                .expect("initialized ledger")
                .initial_materialization_consumed_bytes
        };
        assert!(
            initial > 0,
            "PPT media must be retained in the shared charge"
        );

        let (_, rebuilt_images, rebuilt_allowed) =
            process_file_attachments(std::slice::from_ref(&attachment), 8_000, &resources);
        assert!(rebuilt_allowed);
        assert_eq!(rebuilt_images.len(), 1);
        {
            let ledger = resources[0].turn_budget.ledger.lock().expect("ledger");
            assert_eq!(
                ledger
                    .as_ref()
                    .expect("initialized ledger")
                    .initial_materialization_consumed_bytes,
                initial,
                "provider/profile rebuild must not double-charge the initial message"
            );
        }

        let ctx = crate::tools::ToolExecContext {
            session_id: Some("session-1".into()),
            turn_id: Some("turn-1".into()),
            agent_id: Some("agent-1".into()),
            context_resource_refs: resources.clone(),
            ..Default::default()
        };
        crate::tools::tool_read_context_resource(
            &json!({
                "resource_ref": "resource_ref_0",
                "mode": "base64",
                "limit": 1024
            }),
            &ctx,
        )
        .await
        .expect("bounded continuation");
        let ledger = resources[0].turn_budget.ledger.lock().expect("ledger");
        let ledger = ledger.as_ref().expect("initialized ledger");
        assert_eq!(ledger.initial_materialization_consumed_bytes, initial);
        assert!(ledger.continuation_consumed_bytes > 0);
        assert!(
            ledger.initial_materialization_consumed_bytes + ledger.continuation_consumed_bytes
                <= ledger.baseline_remaining_bytes
        );
    }

    #[test]
    fn invalid_or_poisoned_turn_ledger_never_delivers_typed_content_or_media() {
        const SENTINEL: &str = "PRIVATE-TYPED-CONTENT-SENTINEL";
        let text_attachment = Attachment {
            name: "private.txt".into(),
            mime_type: "text/plain".into(),
            source: Some("mention".into()),
            data: Some(base64::engine::general_purpose::STANDARD.encode(SENTINEL)),
            file_path: Some("/immutable/private.txt".into()),
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_role: None,
            quote_project_root: None,
            quote_worktree_root: None,
        };
        let (text, images, allowed) =
            process_file_attachments(std::slice::from_ref(&text_attachment), 8_000, &[]);
        assert!(!allowed);
        assert!(images.is_empty());
        assert!(text.contains("materialization=\"reference\""));
        assert!(!text.contains(SENTINEL));

        let attachments = vec![text_attachment.clone(), text_attachment.clone()];
        let mut split_resources = context_resources(&attachments);
        split_resources[1].turn_budget =
            Arc::new(crate::prompt_context::ContextResourceTurnBudget::default());
        let (text, images, allowed) =
            process_file_attachments(&attachments, 8_000, &split_resources);
        assert!(!allowed);
        assert!(images.is_empty());
        assert!(!text.contains(SENTINEL));

        let poisoned_resources = context_resources(std::slice::from_ref(&text_attachment));
        let poisoned_owner = poisoned_resources[0].turn_budget.clone();
        let _ = std::panic::catch_unwind(|| {
            let _guard = poisoned_owner.ledger.lock().expect("unpoisoned ledger");
            panic!("poison test ledger");
        });
        let (text, images, allowed) = process_file_attachments(
            std::slice::from_ref(&text_attachment),
            8_000,
            &poisoned_resources,
        );
        assert!(!allowed);
        assert!(images.is_empty());
        assert!(!text.contains(SENTINEL));
    }

    #[test]
    fn invalid_typed_ledger_preserves_ordinary_text_and_extracted_images() {
        const SENTINEL: &str = "PRIVATE-MIXED-TYPED-SENTINEL";
        let root = tempfile::tempdir().expect("tempdir");
        let ordinary_path = root.path().join("ordinary.pptx");
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file("ppt/slides/slide1.xml", options)
            .expect("slide entry");
        writer
            .write_all(b"<p:sld><a:p><a:r><a:t>ordinary-visible</a:t></a:r></a:p></p:sld>")
            .expect("slide XML");
        writer
            .start_file("ppt/media/image1.png", options)
            .expect("media entry");
        writer.write_all(b"ordinary-image").expect("media bytes");
        std::fs::write(&ordinary_path, writer.finish().expect("PPTX").into_inner())
            .expect("ordinary PPTX");
        let attachments = vec![
            Attachment {
                name: "typed.txt".into(),
                mime_type: "text/plain".into(),
                source: Some("mention".into()),
                data: Some(base64::engine::general_purpose::STANDARD.encode(SENTINEL)),
                file_path: Some("/immutable/typed.txt".into()),
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            },
            Attachment {
                name: "ordinary.pptx".into(),
                mime_type:
                    "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                        .into(),
                source: Some("upload".into()),
                data: None,
                file_path: Some(ordinary_path.to_string_lossy().into_owned()),
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            },
        ];

        let (text, images, allowed) = process_file_attachments(&attachments, 8_000, &[]);
        assert!(!allowed);
        assert!(text.contains("ordinary-visible"));
        assert!(!text.contains(SENTINEL));
        assert_eq!(images.len(), 1, "ordinary PPT media must remain available");
    }

    #[tokio::test]
    async fn unsupported_typed_reference_keeps_a_small_exact_base64_continuation() {
        let attachment = Attachment {
            name: "paper.pdf".into(),
            mime_type: "application/pdf".into(),
            source: Some("mention".into()),
            data: Some(base64::engine::general_purpose::STANDARD.encode(b"%PDF-frozen")),
            file_path: Some("/immutable/paper.pdf".into()),
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_role: None,
            quote_project_root: None,
            quote_worktree_root: None,
        };
        let resources = context_resources(std::slice::from_ref(&attachment));
        let (text, images, allowed) =
            process_file_attachments(std::slice::from_ref(&attachment), 8_000, &resources);
        assert!(allowed);
        assert!(images.is_empty());
        assert!(text.contains("materialization=\"reference\""));

        let ctx = crate::tools::ToolExecContext {
            session_id: Some("session-1".into()),
            turn_id: Some("turn-1".into()),
            agent_id: Some("agent-1".into()),
            context_resource_refs: resources,
            ..Default::default()
        };
        crate::tools::tool_read_context_resource(
            &json!({
                "resource_ref": "resource_ref_0",
                "mode": "base64",
                "limit": 1
            }),
            &ctx,
        )
        .await
        .expect("the batch-global continuation floor must fund one exact byte");
    }
}
