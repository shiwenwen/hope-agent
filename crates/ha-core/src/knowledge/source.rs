//! Raw-source inbox for Knowledge Compiler Phase 1.
//!
//! Sources are Hope-managed input snapshots, not editable wiki notes. They are
//! stored under `~/.hope-agent/knowledge/{kb}/sources/`, with metadata in
//! `sessions.db` via [`KnowledgeRegistry`]. Their chunks are separate from
//! `note_chunk`, so raw material never pollutes compiled-note retrieval.

use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose, Engine as _};
use futures_util::StreamExt;
use serde_json::Value;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use super::types::{
    KnowledgeSource, KnowledgeSourceChunk, KnowledgeSourceImportInput, KnowledgeSourceKind,
    KnowledgeSourceReadResult, KnowledgeSourceStatus,
};

const MAX_DIRECT_SOURCE_BYTES: usize = 5 * 1024 * 1024;
/// Decoded bytes accepted for uploaded PDF/DOCX source imports. HTTP routes
/// add a larger JSON body cap for base64 expansion, but this is the real
/// product limit.
pub const MAX_BINARY_SOURCE_BYTES: usize = 24 * 1024 * 1024;
const MAX_URL_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const SOURCE_CHUNK_CHARS: usize = 4_000;
const USER_AGENT: &str =
    "HopeAgent/KnowledgeSourceImporter (+https://github.com/shiwenwen/hope-agent)";

fn registry() -> Result<&'static std::sync::Arc<super::KnowledgeRegistry>> {
    crate::get_knowledge_db().ok_or_else(|| anyhow!("knowledge db not initialized"))
}

/// Import one raw source into a KB. Exactly one of `content`, `dataBase64`, or
/// `url` is used.
pub async fn import_source(
    kb_id: &str,
    input: KnowledgeSourceImportInput,
) -> Result<KnowledgeSource> {
    // Ensure the KB exists up front so a source import cannot create orphan
    // files in an arbitrary id-shaped directory.
    let kb = registry()?
        .get(kb_id)?
        .ok_or_else(|| anyhow!("knowledge base not found: {kb_id}"))?;
    if kb.archived {
        bail!("cannot import source into archived knowledge base: {kb_id}");
    }

    let imported = match normalize_import_input(input)? {
        NormalizedImport::Url { url, title } => import_url_snapshot(kb_id, &url, title).await?,
        NormalizedImport::Content {
            kind,
            title,
            file_name,
            content,
        } => import_text_snapshot(kb_id, kind, title, file_name, content)?,
        NormalizedImport::File {
            kind,
            title,
            file_name,
            mime_type,
            bytes,
        } => import_file_snapshot(kb_id, kind, title, file_name, mime_type, bytes)?,
    };

    emit(kb_id, "source_import");
    Ok(imported)
}

pub fn list_sources(kb_id: &str) -> Result<Vec<KnowledgeSource>> {
    ensure_kb_exists(kb_id)?;
    registry()?.list_sources(kb_id)
}

pub fn read_source(kb_id: &str, source_id: &str) -> Result<KnowledgeSourceReadResult> {
    let source = registry()?
        .get_source(kb_id, source_id)?
        .ok_or_else(|| anyhow!("source not found: {source_id}"))?;
    let path = source_path(kb_id, &source.stored_path)?;
    let bytes = std::fs::read(&path)?;
    let content = String::from_utf8_lossy(&bytes).to_string();
    Ok(KnowledgeSourceReadResult { source, content })
}

pub fn reextract_source(kb_id: &str, source_id: &str) -> Result<KnowledgeSource> {
    let source = registry()?
        .get_source(kb_id, source_id)?
        .ok_or_else(|| anyhow!("source not found: {source_id}"))?;
    let path = source_path(kb_id, &source.stored_path)?;
    let bytes = std::fs::read(&path)?;
    let content = String::from_utf8_lossy(&bytes).to_string();
    let content_hash = super::blake3_hex(content.as_bytes());
    let chunks = build_chunks(source_id, &content);
    let updated = registry()?
        .replace_source_chunks(
            kb_id,
            source_id,
            &content_hash,
            Some(&content_hash),
            content.as_bytes().len() as i64,
            &chunks,
        )?
        .ok_or_else(|| anyhow!("source not found during reextract: {source_id}"))?;
    emit(kb_id, "source_reextract");
    Ok(updated)
}

pub fn delete_source(kb_id: &str, source_id: &str) -> Result<bool> {
    ensure_kb_exists(kb_id)?;
    let Some(stored_path) = registry()?.delete_source(kb_id, source_id)? else {
        return Ok(false);
    };
    let path = source_path(kb_id, &stored_path)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    emit(kb_id, "source_delete");
    Ok(true)
}

fn import_text_snapshot(
    kb_id: &str,
    kind: KnowledgeSourceKind,
    title: Option<String>,
    file_name: Option<String>,
    content: String,
) -> Result<KnowledgeSource> {
    if content.as_bytes().len() > MAX_DIRECT_SOURCE_BYTES {
        bail!(
            "source is too large ({} bytes, max {})",
            content.as_bytes().len(),
            MAX_DIRECT_SOURCE_BYTES
        );
    }
    let title = choose_title(title, file_name.as_deref(), None);
    let ext = match kind {
        KnowledgeSourceKind::Markdown => "md",
        KnowledgeSourceKind::Pdf | KnowledgeSourceKind::Docx => "md",
        KnowledgeSourceKind::Text | KnowledgeSourceKind::UrlSnapshot => "txt",
    };
    persist_source(kb_id, kind, title, None, ext, content)
}

fn import_file_snapshot(
    kb_id: &str,
    kind: KnowledgeSourceKind,
    title: Option<String>,
    file_name: Option<String>,
    mime_type: Option<String>,
    bytes: Vec<u8>,
) -> Result<KnowledgeSource> {
    if bytes.len() > MAX_BINARY_SOURCE_BYTES {
        bail!(
            "source file is too large ({} bytes, max {})",
            bytes.len(),
            MAX_BINARY_SOURCE_BYTES
        );
    }

    let title = choose_title(title, file_name.as_deref(), None);
    match kind {
        KnowledgeSourceKind::Markdown | KnowledgeSourceKind::Text => {
            let content = String::from_utf8_lossy(&bytes).to_string();
            import_text_snapshot(kb_id, kind, Some(title), file_name, content)
        }
        KnowledgeSourceKind::Pdf | KnowledgeSourceKind::Docx => {
            let file_name = file_name.unwrap_or_else(|| default_file_name(kind).to_string());
            let mime = mime_type.unwrap_or_else(|| default_mime_type(kind).to_string());
            let extracted = extract_uploaded_document(kind, &file_name, &mime, &bytes)?;
            let imported_at = chrono::Utc::now().to_rfc3339();
            let mut snapshot = format!(
                "# {title}\n\nSource: {file_name}\nImported: {imported_at}\nSource-Type: {}\nContent-Type: {mime}\nOriginal-Bytes: {}\n\n---\n\n",
                kind.as_str(),
                bytes.len()
            );
            snapshot.push_str(extracted.trim());
            snapshot.push('\n');

            persist_source(
                kb_id,
                kind,
                title,
                Some(format!("local-file:{file_name}")),
                "md",
                snapshot,
            )
        }
        KnowledgeSourceKind::UrlSnapshot => bail!("url_snapshot source imports require url"),
    }
}

enum NormalizedImport {
    Url {
        url: String,
        title: Option<String>,
    },
    Content {
        kind: KnowledgeSourceKind,
        title: Option<String>,
        file_name: Option<String>,
        content: String,
    },
    File {
        kind: KnowledgeSourceKind,
        title: Option<String>,
        file_name: Option<String>,
        mime_type: Option<String>,
        bytes: Vec<u8>,
    },
}

fn normalize_import_input(input: KnowledgeSourceImportInput) -> Result<NormalizedImport> {
    let url = normalize_optional_owned(input.url);
    let content = normalize_content_owned(input.content);
    let data_base64 = normalize_optional_owned(input.data_base64);
    let supplied = url.is_some() as u8 + content.is_some() as u8 + data_base64.is_some() as u8;
    if supplied != 1 {
        bail!("source import accepts exactly one of content, dataBase64, or url");
    }

    if let Some(url) = url {
        return Ok(NormalizedImport::Url {
            url,
            title: input.title,
        });
    }

    if let Some(content) = content {
        let kind = input.kind.unwrap_or_else(|| infer_kind(&input.file_name));
        if matches!(kind, KnowledgeSourceKind::UrlSnapshot) {
            bail!("url_snapshot source imports require url");
        }
        if matches!(kind, KnowledgeSourceKind::Pdf | KnowledgeSourceKind::Docx) {
            bail!("pdf/docx source imports require dataBase64");
        }
        return Ok(NormalizedImport::Content {
            kind,
            title: input.title,
            file_name: input.file_name,
            content,
        });
    }

    let data_base64 = data_base64.expect("checked exactly one import payload");
    let kind = input.kind.unwrap_or_else(|| infer_kind(&input.file_name));
    if matches!(kind, KnowledgeSourceKind::UrlSnapshot) {
        bail!("url_snapshot source imports require url");
    }
    let bytes = decode_base64_source(&data_base64)?;
    Ok(NormalizedImport::File {
        kind,
        title: input.title,
        file_name: input.file_name,
        mime_type: normalize_optional_owned(input.mime_type),
        bytes,
    })
}

fn decode_base64_source(raw: &str) -> Result<Vec<u8>> {
    let encoded = raw
        .trim()
        .split_once(',')
        .filter(|(prefix, _)| prefix.trim_start().starts_with("data:"))
        .map(|(_, payload)| payload)
        .unwrap_or_else(|| raw.trim());
    let bytes = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| anyhow!("invalid source file base64: {e}"))?;
    if bytes.is_empty() {
        bail!("source file is empty");
    }
    if bytes.len() > MAX_BINARY_SOURCE_BYTES {
        bail!(
            "source file is too large ({} bytes, max {})",
            bytes.len(),
            MAX_BINARY_SOURCE_BYTES
        );
    }
    Ok(bytes)
}

fn extract_uploaded_document(
    kind: KnowledgeSourceKind,
    file_name: &str,
    mime_type: &str,
    bytes: &[u8],
) -> Result<String> {
    let suffix = match kind {
        KnowledgeSourceKind::Pdf => ".pdf",
        KnowledgeSourceKind::Docx => ".docx",
        _ => bail!("only PDF and DOCX source files require extraction"),
    };
    let mut tmp = tempfile::Builder::new()
        .prefix("ha_kb_source_")
        .suffix(suffix)
        .tempfile()?;
    tmp.write_all(bytes)?;
    tmp.flush()?;

    let path = tmp.path().to_string_lossy().to_string();
    let extracted = crate::file_extract::extract(&path, file_name, mime_type);
    let Some(text) = extracted.text else {
        bail!("source file has no extractable text");
    };
    if let Some(msg) = text
        .strip_prefix("[Error extracting content:")
        .and_then(|s| s.strip_suffix(']'))
    {
        bail!("source file extraction failed: {}", msg.trim());
    }
    let text = text.trim().to_string();
    if text.is_empty() {
        bail!("source file has no extractable text");
    }
    Ok(text)
}

async fn import_url_snapshot(
    kb_id: &str,
    url: &str,
    requested_title: Option<String>,
) -> Result<KnowledgeSource> {
    let cfg = crate::config::cached_config();
    let ssrf_cfg = cfg.ssrf.clone();
    let web_cfg = cfg.web_fetch.clone();
    let effective_policy = if web_cfg.ssrf_protection {
        ssrf_cfg.web_fetch()
    } else {
        crate::security::ssrf::SsrfPolicy::AllowPrivate
    };
    let trusted_hosts = ssrf_cfg.trusted_hosts.clone();
    let parsed = crate::security::ssrf::check_url(url, effective_policy, &trusted_hosts).await?;

    let max_redirects = web_cfg.max_redirects;
    let timeout_seconds = web_cfg.timeout_seconds.max(1);
    let user_agent = if web_cfg.user_agent.trim().is_empty() {
        USER_AGENT.to_string()
    } else {
        web_cfg.user_agent.clone()
    };
    let redirect_policy_hosts = trusted_hosts.clone();
    let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= max_redirects {
            return attempt.error("too many redirects");
        }
        if let Some(host) = attempt.url().host_str() {
            if crate::security::ssrf::check_host_blocking_sync(
                host,
                effective_policy,
                &redirect_policy_hosts,
            ) {
                return attempt.stop();
            }
        }
        attempt.follow()
    });

    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(timeout_seconds))
            .redirect(redirect_policy),
    )
    .build()
    .map_err(|e| anyhow!("failed to create HTTP client: {e}"))?;

    let resp = client
        .get(parsed.clone())
        .send()
        .await
        .map_err(|e| anyhow!("source URL fetch failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("source URL returned HTTP {}", status.as_u16());
    }

    let final_url = resp.url().to_string();
    crate::security::ssrf::check_url(&final_url, effective_policy, &trusted_hosts).await?;
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut body_bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("source URL stream read failed: {e}"))?;
        body_bytes.extend_from_slice(&chunk);
        if body_bytes.len() > MAX_URL_RESPONSE_BYTES {
            body_bytes.truncate(MAX_URL_RESPONSE_BYTES);
            truncated = true;
            break;
        }
    }
    let body = String::from_utf8_lossy(&body_bytes).to_string();
    let (text, extracted_title) = extract_snapshot_text(&body, &content_type, &final_url);
    let title = choose_title(requested_title, None, extracted_title.as_deref());
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let mut snapshot = format!(
        "# {title}\n\nSource: {final_url}\nFetched: {fetched_at}\nContent-Type: {content_type}\n"
    );
    if truncated {
        snapshot.push_str("Truncated: true\n");
    }
    snapshot.push_str("\n---\n\n");
    snapshot.push_str(text.trim());
    snapshot.push('\n');

    persist_source(
        kb_id,
        KnowledgeSourceKind::UrlSnapshot,
        title,
        Some(final_url),
        "md",
        snapshot,
    )
}

fn persist_source(
    kb_id: &str,
    kind: KnowledgeSourceKind,
    title: String,
    origin_uri: Option<String>,
    ext: &str,
    content: String,
) -> Result<KnowledgeSource> {
    let id = uuid::Uuid::new_v4().to_string();
    let stored_path = format!("{id}.{}", sanitize_ext(ext));
    let dir = source_dir(kb_id)?;
    let path = dir.join(&stored_path);
    crate::platform::write_atomic(&path, content.as_bytes())?;

    let now = chrono::Utc::now().timestamp_millis();
    let content_hash = super::blake3_hex(content.as_bytes());
    let chunks = build_chunks(&id, &content);
    let source = KnowledgeSource {
        id,
        kb_id: kb_id.to_string(),
        kind,
        title,
        origin_uri,
        stored_path,
        content_hash,
        extracted_text_hash: Some(super::blake3_hex(content.as_bytes())),
        status: KnowledgeSourceStatus::Ready,
        compiled_at: None,
        created_at: now,
        updated_at: now,
        size: content.as_bytes().len() as i64,
        chunk_count: chunks.len() as u32,
    };
    if let Err(e) = registry().and_then(|reg| reg.insert_source(&source, &chunks)) {
        if let Err(cleanup_err) = std::fs::remove_file(&path) {
            crate::app_warn!(
                "knowledge",
                "source_import",
                "cleanup orphan source file {} failed after registry insert error: {}",
                path.display(),
                cleanup_err
            );
        }
        return Err(e);
    }
    Ok(source)
}

fn build_chunks(source_id: &str, content: &str) -> Vec<KnowledgeSourceChunk> {
    let chars: Vec<char> = content.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut idx = 0i64;
    while start < chars.len() {
        let end = (start + SOURCE_CHUNK_CHARS).min(chars.len());
        let body: String = chars[start..end].iter().collect();
        chunks.push(KnowledgeSourceChunk {
            id: 0,
            source_id: source_id.to_string(),
            chunk_index: idx,
            body: body.clone(),
            start_offset: start as u32,
            end_offset: end as u32,
            content_hash: super::blake3_hex(body.as_bytes()),
        });
        idx += 1;
        start = end;
    }
    chunks
}

fn source_dir(kb_id: &str) -> Result<PathBuf> {
    let dir = crate::paths::knowledge_kb_sources_dir(kb_id)?;
    let path = crate::util::ensure_dir_canonical(&dir)?;
    Ok(PathBuf::from(path))
}

fn source_path(kb_id: &str, stored_path: &str) -> Result<PathBuf> {
    let stored = Path::new(stored_path);
    if stored.is_absolute()
        || stored.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("invalid source stored path");
    }
    let dir = source_dir(kb_id)?;
    let path = dir.join(stored);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("invalid source path"))?
        .canonicalize()?;
    if !parent.starts_with(&dir) {
        bail!("source path escapes source directory");
    }
    Ok(path)
}

fn ensure_kb_exists(kb_id: &str) -> Result<()> {
    registry()?
        .get(kb_id)?
        .map(|_| ())
        .ok_or_else(|| anyhow!("knowledge base not found: {kb_id}"))
}

fn infer_kind(file_name: &Option<String>) -> KnowledgeSourceKind {
    let Some(name) = file_name.as_deref() else {
        return KnowledgeSourceKind::Text;
    };
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        KnowledgeSourceKind::Markdown
    } else if lower.ends_with(".pdf") {
        KnowledgeSourceKind::Pdf
    } else if lower.ends_with(".docx") {
        KnowledgeSourceKind::Docx
    } else {
        KnowledgeSourceKind::Text
    }
}

fn default_file_name(kind: KnowledgeSourceKind) -> &'static str {
    match kind {
        KnowledgeSourceKind::Pdf => "source.pdf",
        KnowledgeSourceKind::Docx => "source.docx",
        KnowledgeSourceKind::Markdown => "source.md",
        KnowledgeSourceKind::UrlSnapshot => "source.md",
        KnowledgeSourceKind::Text => "source.txt",
    }
}

fn default_mime_type(kind: KnowledgeSourceKind) -> &'static str {
    match kind {
        KnowledgeSourceKind::Pdf => "application/pdf",
        KnowledgeSourceKind::Docx => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        KnowledgeSourceKind::Markdown | KnowledgeSourceKind::UrlSnapshot => "text/markdown",
        KnowledgeSourceKind::Text => "text/plain",
    }
}

fn choose_title(
    requested: Option<String>,
    file_name: Option<&str>,
    extracted: Option<&str>,
) -> String {
    for candidate in [requested.as_deref(), extracted, file_name] {
        if let Some(value) = normalize_optional(candidate) {
            return crate::truncate_utf8(value, 120).to_string();
        }
    }
    "Untitled source".to_string()
}

fn normalize_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn normalize_optional_owned(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_content_owned(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn sanitize_ext(ext: &str) -> &'static str {
    match ext {
        "md" | "markdown" => "md",
        _ => "txt",
    }
}

fn extract_snapshot_text(body: &str, content_type: &str, url: &str) -> (String, Option<String>) {
    let content_type = content_type.to_ascii_lowercase();
    if content_type.contains("text/html") || looks_like_html(body) {
        let parsed_url = url::Url::parse(url)
            .unwrap_or_else(|_| url::Url::parse("https://example.com").unwrap());
        if let Ok(product) = readability::extractor::extract(&mut body.as_bytes(), &parsed_url) {
            let title = if product.title.trim().is_empty() {
                None
            } else {
                Some(product.title)
            };
            if !product.content.trim().is_empty() {
                let md = htmd::convert(&product.content)
                    .unwrap_or_else(|_| strip_html_tags(&product.content));
                return (md, title);
            }
        }
        return (
            htmd::convert(body).unwrap_or_else(|_| strip_html_tags(body)),
            extract_title_tag(body),
        );
    }
    if content_type.contains("application/json") {
        if let Ok(value) = serde_json::from_str::<Value>(body) {
            if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                return (pretty, None);
            }
        }
    }
    (body.to_string(), None)
}

fn looks_like_html(body: &str) -> bool {
    let sample = body
        .trim_start()
        .chars()
        .take(256)
        .collect::<String>()
        .to_ascii_lowercase();
    sample.starts_with("<!doctype")
        || sample.starts_with("<html")
        || sample.contains("<body")
        || sample.contains("<article")
}

fn extract_title_tag(html: &str) -> Option<String> {
    let re = regex::Regex::new("(?is)<title[^>]*>(.*?)</title>").ok()?;
    let raw = re.captures(html)?.get(1)?.as_str();
    let text = strip_html_tags(raw);
    normalize_optional(Some(&text)).map(str::to_string)
}

fn strip_html_tags(html: &str) -> String {
    let re = regex::Regex::new("(?is)<script[^>]*>.*?</script>|<style[^>]*>.*?</style>|<[^>]+>")
        .expect("valid html stripping regex");
    let stripped = re.replace_all(html, " ");
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn emit(kb_id: &str, op: &str) {
    if let Some(bus) = crate::get_event_bus() {
        let _ = bus.emit(
            "knowledge:changed",
            serde_json::json!({ "kbId": kb_id, "op": op }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose, Engine as _};

    fn input() -> KnowledgeSourceImportInput {
        KnowledgeSourceImportInput {
            kind: None,
            title: None,
            file_name: None,
            mime_type: None,
            content: None,
            data_base64: None,
            url: None,
        }
    }

    #[test]
    fn normalize_import_rejects_ambiguous_url_and_content() {
        let mut req = input();
        req.url = Some("https://example.com".to_string());
        req.content = Some("body".to_string());

        assert!(normalize_import_input(req).is_err());
    }

    #[test]
    fn normalize_import_preserves_source_content_bytes() {
        let mut req = input();
        req.file_name = Some("note.md".to_string());
        req.content = Some("\n  body  \n".to_string());

        let NormalizedImport::Content { kind, content, .. } =
            normalize_import_input(req).expect("valid content import")
        else {
            panic!("expected content import");
        };

        assert_eq!(kind, KnowledgeSourceKind::Markdown);
        assert_eq!(content, "\n  body  \n");
    }

    #[test]
    fn normalize_import_rejects_url_snapshot_without_url() {
        let mut req = input();
        req.kind = Some(KnowledgeSourceKind::UrlSnapshot);
        req.content = Some("body".to_string());

        assert!(normalize_import_input(req).is_err());
    }

    #[test]
    fn normalize_import_rejects_pdf_content_without_file_bytes() {
        let mut req = input();
        req.kind = Some(KnowledgeSourceKind::Pdf);
        req.file_name = Some("paper.pdf".to_string());
        req.content = Some("plain text pretending to be extracted pdf".to_string());

        assert!(normalize_import_input(req).is_err());
    }

    #[test]
    fn normalize_import_accepts_uploaded_pdf_bytes() {
        let mut req = input();
        req.file_name = Some("paper.pdf".to_string());
        req.mime_type = Some("application/pdf".to_string());
        req.data_base64 = Some(general_purpose::STANDARD.encode(b"%PDF"));

        let NormalizedImport::File {
            kind,
            file_name,
            mime_type,
            bytes,
            ..
        } = normalize_import_input(req).expect("valid file import")
        else {
            panic!("expected file import");
        };

        assert_eq!(kind, KnowledgeSourceKind::Pdf);
        assert_eq!(file_name.as_deref(), Some("paper.pdf"));
        assert_eq!(mime_type.as_deref(), Some("application/pdf"));
        assert_eq!(bytes, b"%PDF");
    }

    #[test]
    fn decode_base64_source_accepts_data_url_prefix() {
        let encoded = format!(
            "data:application/pdf;base64,{}",
            general_purpose::STANDARD.encode(b"hello")
        );
        assert_eq!(decode_base64_source(&encoded).unwrap(), b"hello");
    }

    #[test]
    fn infer_kind_detects_docx() {
        assert_eq!(
            infer_kind(&Some("Brief.DOCX".to_string())),
            KnowledgeSourceKind::Docx
        );
    }
}
