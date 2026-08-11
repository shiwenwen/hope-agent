//! Read-only access to immutable resources bound by typed turn mentions.
//!
//! The handle is deliberately not a path. The bytes were frozen before the
//! first provider attempt and are scoped to the exact session, turn, and agent
//! principal. This keeps continuation deterministic across retries while
//! preventing copied refs from becoming ambient capabilities.

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};

use super::ToolExecContext;

const DEFAULT_TEXT_LINES: usize = 1_000;
const MAX_TEXT_LINES: usize = 5_000;
const MAX_TEXT_PAGE_BYTES: usize = 64 * 1024;
const DEFAULT_BASE64_BYTES: usize = 32 * 1024;
const MAX_BASE64_BYTES: usize = 64 * 1024;

pub(crate) async fn tool_read_context_resource(
    args: &Value,
    ctx: &ToolExecContext,
) -> Result<String> {
    let resource_ref = args
        .get("resource_ref")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing resource_ref"))?;
    let resource = ctx
        .context_resource_refs
        .iter()
        .find(|candidate| candidate.resource_ref == resource_ref)
        .ok_or_else(|| anyhow::anyhow!("context resource is unavailable in this turn"))?;

    if ctx.session_id.as_deref() != Some(resource.parent_session_id.as_str())
        || ctx.turn_id.as_deref() != resource.parent_turn_id.as_deref()
        || ctx.agent_id.as_deref() != Some(resource.principal_agent_id.as_str())
    {
        bail!("context resource is outside the current turn or principal scope");
    }

    // Every successful result remains in canonical/provider history. Hold the
    // per-turn ledger while executing this serial tool so pagination and image
    // retries cannot each spend a fresh copy of the 256 MiB allowance.
    let baseline = context_extraction_budget(ctx)?.remaining_bytes();
    let resource_refs = ctx
        .context_resource_refs
        .iter()
        .map(|resource| resource.resource_ref.clone())
        .collect::<Vec<_>>();
    if ctx
        .context_resource_refs
        .iter()
        .any(|candidate| !std::sync::Arc::ptr_eq(&candidate.turn_budget, &resource.turn_budget))
    {
        bail!("context resources do not share one turn-owned budget ledger");
    }
    let mut turn_budget = resource
        .turn_budget
        .ledger
        .lock()
        .map_err(|_| anyhow::anyhow!("context resource turn budget ledger is poisoned"))?;
    let turn_budget =
        turn_budget.get_or_insert_with(|| crate::prompt_context::ContextResourceBudgetLedger {
            baseline_remaining_bytes: baseline,
            initial_materialization_consumed_bytes: 0,
            continuation_consumed_bytes: 0,
            resource_refs: resource_refs.clone(),
        });
    if turn_budget.baseline_remaining_bytes != baseline
        || turn_budget.resource_refs != resource_refs
    {
        bail!("context resource turn budget does not match the current frozen resource set");
    }
    let total_consumed = turn_budget
        .initial_materialization_consumed_bytes
        .checked_add(turn_budget.continuation_consumed_bytes)
        .context("context resource turn budget accounting overflow")?;
    let available = turn_budget
        .baseline_remaining_bytes
        .checked_sub(total_consumed)
        .context("context resource turn budget is exhausted")?;
    let mut budget = crate::file_extract::FrozenExtractionBudget::new(available);
    let before = budget.remaining_bytes();
    let result = match args.get("mode").and_then(Value::as_str).unwrap_or("auto") {
        "base64" => read_base64(resource, args, &mut budget),
        "text" => {
            if resource.mime_type.starts_with("image/") {
                bail!("image resources do not have a text view; use mode=auto for bounded vision delivery or mode=base64 for exact byte paging");
            }
            read_text(resource, args, &mut budget)
        }
        "auto" => {
            if resource.mime_type.starts_with("image/") {
                read_image(resource, &mut budget)
            } else {
                read_text(resource, args, &mut budget)
            }
        }
        _ => bail!("mode must be one of auto, text, or base64"),
    };
    if result.is_ok() {
        let consumed = before
            .checked_sub(budget.remaining_bytes())
            .context("context resource turn budget accounting underflow")?;
        turn_budget.continuation_consumed_bytes = turn_budget
            .continuation_consumed_bytes
            .checked_add(consumed)
            .context("context resource turn budget accounting overflow")?;
    }
    result
}

fn read_text(
    resource: &crate::prompt_context::ContextResourceRef,
    args: &Value,
    budget: &mut crate::file_extract::FrozenExtractionBudget,
) -> Result<String> {
    let offset = usize_arg(args, "offset", 1)?.max(1);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TEXT_LINES as u64)
        .clamp(1, MAX_TEXT_LINES as u64);
    let limit = usize::try_from(limit).context("text line limit exceeds this platform")?;
    let byte_offset = usize_arg(args, "byte_offset", 0)?;

    let extracted;
    let extraction_truncated;
    let text = if crate::file_extract::is_text_like(&resource.mime_type, &resource.file_name) {
        extraction_truncated = false;
        budget.retain_provider_text(MAX_TEXT_PAGE_BYTES)?;
        std::str::from_utf8(&resource.bytes).map_err(|_| {
            anyhow::anyhow!("frozen text resource is not valid UTF-8; use base64 mode")
        })?
    } else {
        let content = crate::file_extract::extract_frozen_bytes_bounded(
            &resource.bytes,
            &resource.file_name,
            &resource.mime_type,
            budget,
            false,
            MAX_TEXT_PAGE_BYTES,
        )
        .with_context(|| {
            "bounded text extraction failed; use mode=base64 with offset/limit to inspect the exact frozen bytes"
        })?;
        extraction_truncated = content.extraction_truncated;
        extracted = content.text.unwrap_or_default();
        extracted.as_str()
    };
    let page = read_text_page_bounded(text, offset, byte_offset, limit)?;
    Ok(json!({
        "resourceRef": resource.resource_ref,
        "fileName": resource.file_name,
        "mimeType": resource.mime_type,
        "sourceBytes": resource.bytes.len(),
        "mode": "text",
        "offset": offset,
        "byteOffset": byte_offset,
        "linesRead": page.lines_read,
        "totalLines": page.total_lines,
        "truncated": page.truncated,
        "nextOffset": page.next_offset,
        "nextByteOffset": page.next_byte_offset,
        "extractionTruncated": extraction_truncated,
        "extractionContinuation": extraction_truncated.then_some("base64"),
        "content": page.content,
    })
    .to_string())
}

fn read_image(
    resource: &crate::prompt_context::ContextResourceRef,
    budget: &mut crate::file_extract::FrozenExtractionBudget,
) -> Result<String> {
    let sniffed_mime = super::read::detect_image_mime(&resource.bytes)
        .context("frozen image payload has no supported image signature; use mode=base64")?;
    let (encoded_upper, transient_bytes) =
        super::read::bounded_image_delivery_projection(&resource.bytes).with_context(|| {
            "frozen image exceeds the bounded vision decode budget; use mode=base64 paging"
        })?;
    budget
        .reserve_extracted_image(encoded_upper, sniffed_mime.len(), transient_bytes)
        .with_context(|| "frozen image exceeds the turn memory ceiling; use mode=base64 paging")?;
    let (encoded, delivered_mime) = super::read::resize_image_if_needed(
        &resource.bytes,
        sniffed_mime,
    )
    .with_context(|| {
        "frozen image could not be decoded within the bounded vision path; use mode=base64 paging"
    })?;
    if encoded.len() > encoded_upper {
        bail!("bounded image output exceeded its preflight projection");
    }
    Ok(super::image_markers::build_image_base64_marker(
        delivered_mime,
        &encoded,
        &format!(
            "Frozen context image {} (resource_ref={}).",
            resource.file_name, resource.resource_ref
        ),
    ))
}

fn context_extraction_budget(
    ctx: &ToolExecContext,
) -> Result<crate::file_extract::FrozenExtractionBudget> {
    let remaining =
        crate::attachments::context_resource_extraction_budget_bytes(&ctx.context_resource_refs)?;
    Ok(crate::file_extract::FrozenExtractionBudget::new(remaining))
}

fn usize_arg(args: &Value, name: &str, default: usize) -> Result<usize> {
    match args.get(name).and_then(Value::as_u64) {
        Some(value) => usize::try_from(value)
            .with_context(|| format!("{name} exceeds this platform's addressable range")),
        None => Ok(default),
    }
}

struct BoundedTextPage {
    content: String,
    lines_read: usize,
    total_lines: usize,
    truncated: bool,
    next_offset: Option<usize>,
    next_byte_offset: Option<usize>,
}

fn read_text_page_bounded(
    text: &str,
    offset: usize,
    byte_offset: usize,
    line_limit: usize,
) -> Result<BoundedTextPage> {
    let total_lines = text.lines().count();
    let mut content = String::with_capacity(MAX_TEXT_PAGE_BYTES.min(text.len()));
    let mut lines_read = 0usize;
    let mut next_offset = None;
    let mut next_byte_offset = None;
    let start_index = offset.saturating_sub(1);

    for (index, line) in text.lines().enumerate().skip(start_index) {
        if lines_read >= line_limit {
            next_offset = Some(index + 1);
            next_byte_offset = Some(0);
            break;
        }
        let start = if index == start_index { byte_offset } else { 0 };
        if start > line.len() || !line.is_char_boundary(start) {
            bail!("text byte_offset must be a UTF-8 boundary within the selected line");
        }
        let remaining = MAX_TEXT_PAGE_BYTES.saturating_sub(content.len());
        let line_tail = &line[start..];
        let mut consumed = 0usize;
        for character in line_tail.chars() {
            let width = character.len_utf8();
            if consumed.saturating_add(width) > remaining {
                break;
            }
            content.push(character);
            consumed += width;
        }
        lines_read += 1;
        if consumed < line_tail.len() {
            next_offset = Some(index + 1);
            next_byte_offset = Some(start.saturating_add(consumed));
            break;
        }
        if content.len() < MAX_TEXT_PAGE_BYTES {
            content.push('\n');
        } else if index + 1 < total_lines {
            next_offset = Some(index + 2);
            next_byte_offset = Some(0);
            break;
        }
    }
    let truncated = next_offset.is_some();
    Ok(BoundedTextPage {
        content,
        lines_read,
        total_lines,
        truncated,
        next_offset,
        next_byte_offset,
    })
}

fn read_base64(
    resource: &crate::prompt_context::ContextResourceRef,
    args: &Value,
    budget: &mut crate::file_extract::FrozenExtractionBudget,
) -> Result<String> {
    let offset = usize_arg(args, "offset", 0)?;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_BASE64_BYTES as u64)
        .clamp(1, MAX_BASE64_BYTES as u64);
    let limit = usize::try_from(limit).context("base64 limit exceeds this platform")?;
    if offset > resource.bytes.len() {
        bail!("base64 byte offset exceeds frozen resource length");
    }
    let end = offset.saturating_add(limit).min(resource.bytes.len());
    let encoded_upper = (end - offset)
        .checked_add(2)
        .and_then(|value| value.checked_div(3))
        .and_then(|value| value.checked_mul(4))
        .context("base64 page size accounting overflow")?;
    budget
        .retain_provider_text(encoded_upper.saturating_add(1_024))
        .with_context(|| {
            "Base64 page exceeds the remaining turn memory budget; request a smaller limit"
        })?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&resource.bytes[offset..end]);
    Ok(json!({
        "resourceRef": resource.resource_ref,
        "fileName": resource.file_name,
        "mimeType": resource.mime_type,
        "sourceBytes": resource.bytes.len(),
        "mode": "base64",
        "byteOffset": offset,
        "bytesRead": end - offset,
        "truncated": end < resource.bytes.len(),
        "nextOffset": (end < resource.bytes.len()).then_some(end),
        "contentBase64": encoded,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};
    use std::sync::Arc;

    use base64::Engine as _;
    use serde_json::json;
    use zip::write::SimpleFileOptions;

    use super::*;

    fn context_resource(file_name: &str, mime_type: &str, bytes: Vec<u8>) -> ToolExecContext {
        ToolExecContext {
            context_window_tokens: Some(8_000),
            session_id: Some("session-1".into()),
            turn_id: Some("turn-1".into()),
            agent_id: Some("agent-1".into()),
            context_resource_refs: vec![crate::prompt_context::ContextResourceRef {
                resource_ref: "resource_ref_test".into(),
                mention_id: "mention-1".into(),
                target_id: format!("notes/{file_name}"),
                file_name: file_name.into(),
                mime_type: mime_type.into(),
                parent_session_id: "session-1".into(),
                parent_turn_id: Some("turn-1".into()),
                principal_agent_id: "agent-1".into(),
                bytes: Arc::from(bytes),
                turn_budget: Arc::new(crate::prompt_context::ContextResourceTurnBudget::default()),
            }],
            ..Default::default()
        }
    }

    fn context() -> ToolExecContext {
        context_resource("demo.txt", "text/plain", b"one\ntwo\nthree\n".to_vec())
    }

    fn docx(text: &str) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file("word/document.xml", options)
            .expect("start DOCX entry");
        write!(
            writer,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"
        )
        .expect("write DOCX XML");
        writer.finish().expect("finish DOCX").into_inner()
    }

    #[tokio::test]
    async fn paginates_the_frozen_snapshot() {
        let result = tool_read_context_resource(
            &json!({"resource_ref": "resource_ref_test", "offset": 2, "limit": 1}),
            &context(),
        )
        .await
        .unwrap();
        assert!(result.contains("two"));
        assert!(result.contains("\"nextOffset\":3"));
    }

    #[tokio::test]
    async fn rejects_a_ref_copied_to_another_turn() {
        let mut ctx = context();
        ctx.turn_id = Some("turn-2".into());
        let error = tool_read_context_resource(&json!({"resource_ref": "resource_ref_test"}), &ctx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("outside the current turn"));
    }

    #[tokio::test]
    async fn long_utf8_line_uses_bounded_byte_continuation_without_line_vec() {
        let text = "界".repeat(30_000);
        let ctx = context_resource("long.sql", "application/octet-stream", text.into_bytes());
        let first = tool_read_context_resource(
            &json!({"resource_ref": "resource_ref_test", "limit": 1}),
            &ctx,
        )
        .await
        .expect("first page");
        let first: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(first["truncated"], true);
        assert!(first["content"].as_str().unwrap().len() <= MAX_TEXT_PAGE_BYTES);
        let byte_offset = first["nextByteOffset"].as_u64().expect("byte continuation");
        assert_eq!(byte_offset % 3, 0, "must stay on a UTF-8 boundary");

        let second = tool_read_context_resource(
            &json!({
                "resource_ref": "resource_ref_test",
                "offset": 1,
                "byte_offset": byte_offset,
                "limit": 1
            }),
            &ctx,
        )
        .await
        .expect("second page");
        let second: Value = serde_json::from_str(&second).unwrap();
        assert!(!second["content"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn office_preview_continues_as_bounded_text_from_same_snapshot() {
        let ctx = context_resource(
            "large.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            docx(&"x".repeat(200_001)),
        );
        let first = tool_read_context_resource(
            &json!({"resource_ref": "resource_ref_test", "limit": 1}),
            &ctx,
        )
        .await
        .expect("Office continuation");
        let first: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(first["mode"], "text");
        assert_eq!(first["extractionTruncated"], true);
        assert_eq!(first["extractionContinuation"], "base64");
        assert_eq!(first["truncated"], true);
        assert!(first["nextByteOffset"].as_u64().is_some());
    }

    #[tokio::test]
    async fn malformed_office_end_name_is_not_reflected_in_tool_errors() {
        let sentinel = "PRIVATE-TOOL-END-NAME-SENTINEL".repeat(100);
        let ctx = context_resource(
            "malformed.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            docx(&format!("safe</{sentinel}>")),
        );
        let error = tool_read_context_resource(&json!({"resource_ref": "resource_ref_test"}), &ctx)
            .await
            .expect_err("mismatched Office XML end tag");
        let error = format!("{error:#}");
        assert!(!error.contains("PRIVATE-TOOL-END-NAME-SENTINEL"));
        assert!(error.len() < 512);
    }

    #[tokio::test]
    async fn pdf_and_legacy_xls_fail_visible_to_exact_base64_paging() {
        for (name, mime) in [
            ("paper.pdf", "application/pdf"),
            ("legacy.xls", "application/vnd.ms-excel"),
        ] {
            let ctx = context_resource(name, mime, b"frozen-binary".to_vec());
            let error =
                tool_read_context_resource(&json!({"resource_ref": "resource_ref_test"}), &ctx)
                    .await
                    .expect_err("auto extraction must be visible");
            assert!(error.to_string().contains("base64"));
            let page = tool_read_context_resource(
                &json!({"resource_ref": "resource_ref_test", "mode": "base64"}),
                &ctx,
            )
            .await
            .expect("base64 page");
            let page: Value = serde_json::from_str(&page).unwrap();
            assert_eq!(
                page["contentBase64"],
                base64::engine::general_purpose::STANDARD.encode(b"frozen-binary")
            );
        }
    }

    #[tokio::test]
    async fn small_valid_image_uses_bounded_vision_and_invalid_or_huge_images_reference_base64() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("PNG");
        let ctx = context_resource("pixel.png", "image/png", png.into_inner());
        let result =
            tool_read_context_resource(&json!({"resource_ref": "resource_ref_test"}), &ctx)
                .await
                .expect("small image");
        assert!(super::super::image_markers::parse_image_markers(&result).is_some());

        let corrupt = context_resource("bad.png", "image/png", b"\x89PNG\r\n\x1a\n".to_vec());
        let error =
            tool_read_context_resource(&json!({"resource_ref": "resource_ref_test"}), &corrupt)
                .await
                .expect_err("corrupt image");
        assert!(error.to_string().contains("base64"));

        let mut huge_bmp = vec![0u8; 54];
        huge_bmp[0..2].copy_from_slice(b"BM");
        huge_bmp[2..6].copy_from_slice(&(54u32).to_le_bytes());
        huge_bmp[10..14].copy_from_slice(&(54u32).to_le_bytes());
        huge_bmp[14..18].copy_from_slice(&(40u32).to_le_bytes());
        huge_bmp[18..22].copy_from_slice(&(100_000i32).to_le_bytes());
        huge_bmp[22..26].copy_from_slice(&(100_000i32).to_le_bytes());
        huge_bmp[26..28].copy_from_slice(&(1u16).to_le_bytes());
        huge_bmp[28..30].copy_from_slice(&(24u16).to_le_bytes());
        let huge = context_resource("huge.bmp", "image/bmp", huge_bmp);
        let error =
            tool_read_context_resource(&json!({"resource_ref": "resource_ref_test"}), &huge)
                .await
                .expect_err("huge dimensions");
        assert!(error.to_string().contains("base64"));
    }

    #[tokio::test]
    async fn successful_pages_share_one_turn_budget_and_new_turn_state_does_not_leak() {
        let bytes = vec![0x5a; MAX_BASE64_BYTES * 100];
        let ctx = context_resource("paged.bin", "application/octet-stream", bytes);
        let rebuilt_ctx = ctx.clone();
        assert!(Arc::ptr_eq(
            &ctx.context_resource_refs[0].turn_budget,
            &rebuilt_ctx.context_resource_refs[0].turn_budget,
        ));
        let mut exhausted = false;
        for page in 0..100usize {
            let offset = (page * MAX_BASE64_BYTES) % ctx.context_resource_refs[0].bytes.len();
            let active_ctx = if page % 2 == 0 { &ctx } else { &rebuilt_ctx };
            match tool_read_context_resource(
                &json!({
                    "resource_ref": "resource_ref_test",
                    "mode": "base64",
                    "offset": offset,
                    "limit": MAX_BASE64_BYTES
                }),
                active_ctx,
            )
            .await
            {
                Ok(_) => {}
                Err(error) => {
                    assert!(error.to_string().contains("memory budget"));
                    exhausted = true;
                    break;
                }
            }
        }
        assert!(
            exhausted,
            "continuation pages must cumulatively exhaust the turn budget"
        );

        // A newly built turn context owns a new Arc ledger; the completed
        // turn's accounting has no process-global lifetime.
        let fresh = context_resource(
            "paged.bin",
            "application/octet-stream",
            vec![0x5a; MAX_BASE64_BYTES * 100],
        );
        tool_read_context_resource(
            &json!({
                "resource_ref": "resource_ref_test",
                "mode": "base64",
                "limit": MAX_BASE64_BYTES
            }),
            &fresh,
        )
        .await
        .expect("fresh turn-owned ledger");
    }
}
