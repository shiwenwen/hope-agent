//! Conversation export — serialize a session's metadata + message history into
//! Markdown / JSON / HTML for download.
//!
//! Single source of truth for both the slash-command (`/export`) and the GUI
//! "Export conversation" dialog. The serializers honour two opt-in switches
//! (`include_thinking`, `include_tools`) so the same code path can produce a
//! lean shareable transcript or a full debugging dump.

use anyhow::Result;
use serde::Serialize;

use crate::session::{MessageRole, SessionDB, SessionMessage, SessionMeta};
use crate::util::html_escape;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Json,
    Html,
}

impl ExportFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "json" => Some(Self::Json),
            "html" => Some(Self::Html),
            _ => None,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Json => "json",
            Self::Html => "html",
        }
    }

    pub fn mime(&self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Json => "application/json; charset=utf-8",
            Self::Html => "text/html; charset=utf-8",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// Include `assistant.thinking` content (chain-of-thought blocks).
    pub include_thinking: bool,
    /// Include `tool_call` arguments + `tool_result` content + `tool_metadata`
    /// (file diff snapshots, etc.).
    pub include_tools: bool,
}

impl ExportOptions {
    /// Lean default: only user/assistant body text, no thinking/tools.
    /// Matches the legacy `/export` slash command output byte-for-byte.
    pub fn lean(format: ExportFormat) -> Self {
        Self {
            format,
            include_thinking: false,
            include_tools: false,
        }
    }

    pub fn full(format: ExportFormat) -> Self {
        Self {
            format,
            include_thinking: true,
            include_tools: true,
        }
    }
}

pub struct ExportPayload {
    pub filename: String,
    pub mime: &'static str,
    pub body: Vec<u8>,
}

/// Turn a session title into a filename stem. Keeps alphanumerics (incl. CJK
/// via `is_alphanumeric`), dash, underscore, space; replaces everything else
/// with `_`. Distinct from [`crate::project::files`]'s `sanitize_filename`,
/// which only strips path separators from already-vetted disk names.
fn title_to_filename_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "session".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn export_session(
    session_db: &SessionDB,
    session_id: &str,
    opts: ExportOptions,
) -> Result<ExportPayload> {
    let meta = session_db
        .get_session(session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?;
    let messages = session_db.load_session_messages(session_id)?;

    let title_for_filename = meta.title.as_deref().unwrap_or("Untitled");
    let filename = format!(
        "{}.{}",
        title_to_filename_stem(title_for_filename),
        opts.format.extension()
    );

    let body = match opts.format {
        ExportFormat::Markdown => to_markdown(&meta, &messages, opts).into_bytes(),
        ExportFormat::Json => to_json(&meta, &messages, opts)?,
        ExportFormat::Html => to_html(&meta, &messages, opts).into_bytes(),
    };

    Ok(ExportPayload {
        filename,
        mime: opts.format.mime(),
        body,
    })
}

// ── Markdown ────────────────────────────────────────────────────────────────

fn to_markdown(meta: &SessionMeta, messages: &[SessionMessage], opts: ExportOptions) -> String {
    let title = meta.title.as_deref().unwrap_or("Untitled");
    let mut out = format!("# {}\n\n", title);

    for msg in messages {
        match msg.role {
            MessageRole::User => md_push_section(&mut out, "User", &msg.content),
            MessageRole::Assistant | MessageRole::TextBlock => {
                md_push_section(&mut out, "Assistant", &msg.content);
            }
            MessageRole::ThinkingBlock => {
                if opts.include_thinking {
                    md_push_thinking(&mut out, &msg.content);
                }
            }
            MessageRole::Tool => {
                if opts.include_tools {
                    md_push_tool(&mut out, msg);
                }
            }
            // Event rows are UI-only system markers; skip in transcripts.
            MessageRole::Event => {}
        }

        // Assistant rows may carry a `thinking` field alongside the body.
        if opts.include_thinking && msg.role == MessageRole::Assistant {
            if let Some(t) = msg.thinking.as_deref() {
                md_push_thinking(&mut out, t);
            }
        }
    }

    out
}

fn md_push_section(out: &mut String, role: &str, content: &str) {
    if content.trim().is_empty() {
        return;
    }
    out.push_str(&format!("## {}\n\n{}\n\n", role, content));
}

fn md_push_thinking(out: &mut String, content: &str) {
    if content.trim().is_empty() {
        return;
    }
    out.push_str(&format!(
        "<details><summary>Thinking</summary>\n\n{}\n\n</details>\n\n",
        content
    ));
}

fn md_push_tool(out: &mut String, msg: &SessionMessage) {
    let name = msg.tool_name.as_deref().unwrap_or("tool");
    out.push_str(&format!("<details><summary>Tool: {}", name));
    if let Some(d) = msg.tool_duration_ms {
        out.push_str(&format!(" ({} ms)", d));
    }
    out.push_str("</summary>\n\n");
    if let Some(args) = msg.tool_arguments.as_deref() {
        out.push_str(&format!("**Arguments**\n\n```json\n{}\n```\n\n", args));
    }
    if let Some(result) = msg.tool_result.as_deref() {
        let lang = if matches!(result.trim_start().chars().next(), Some('{') | Some('[')) {
            "json"
        } else {
            ""
        };
        out.push_str(&format!("**Result**\n\n```{}\n{}\n```\n\n", lang, result));
    }
    if let Some(meta) = msg.tool_metadata.as_deref() {
        let pretty = pretty_json_or_raw(meta);
        out.push_str(&format!("**Metadata**\n\n```json\n{}\n```\n\n", pretty));
    }
    out.push_str("</details>\n\n");
}

/// Pretty-print a JSON string for human-readable Markdown / HTML blocks.
/// Falls back to the raw text when the input isn't valid JSON — tool_metadata
/// is always JSON in practice, but be defensive against legacy rows.
fn pretty_json_or_raw(s: &str) -> String {
    serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| s.to_string())
}

// ── JSON ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct JsonExportEnvelope<'a> {
    session: &'a SessionMeta,
    messages: Vec<serde_json::Value>,
}

fn to_json(
    meta: &SessionMeta,
    messages: &[SessionMessage],
    opts: ExportOptions,
) -> Result<Vec<u8>> {
    let cleaned: Vec<serde_json::Value> = messages
        .iter()
        .filter_map(|m| filter_message_for_json(m, opts))
        .collect();
    let env = JsonExportEnvelope {
        session: meta,
        messages: cleaned,
    };
    Ok(serde_json::to_vec_pretty(&env)?)
}

fn filter_message_for_json(msg: &SessionMessage, opts: ExportOptions) -> Option<serde_json::Value> {
    if matches!(msg.role, MessageRole::Event) {
        return None;
    }
    if matches!(msg.role, MessageRole::Tool) && !opts.include_tools {
        return None;
    }
    if matches!(msg.role, MessageRole::ThinkingBlock) && !opts.include_thinking {
        return None;
    }

    let mut value = serde_json::to_value(msg).ok()?;
    let obj = value.as_object_mut()?;

    if !opts.include_thinking {
        obj.remove("thinking");
    }
    if !opts.include_tools {
        obj.remove("toolCallId");
        obj.remove("toolName");
        obj.remove("toolArguments");
        obj.remove("toolResult");
        obj.remove("toolDurationMs");
        obj.remove("toolMetadata");
    } else {
        // Re-embed JSON-string columns as actual objects so consumers don't
        // have to do a second parse pass.
        re_embed_json_string(obj, "toolArguments");
        re_embed_json_string(obj, "toolMetadata");
        re_embed_json_string(obj, "attachmentsMeta");
    }

    Some(value)
}

fn re_embed_json_string(obj: &mut serde_json::Map<String, serde_json::Value>, key: &str) {
    if let Some(serde_json::Value::String(s)) = obj.get(key).cloned() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&s) {
            obj.insert(key.to_string(), parsed);
        }
    }
}

// ── HTML ────────────────────────────────────────────────────────────────────

fn to_html(meta: &SessionMeta, messages: &[SessionMessage], opts: ExportOptions) -> String {
    let title = meta.title.as_deref().unwrap_or("Untitled");
    let title_esc = html_escape(title);

    let mut body = String::new();
    body.push_str(&format!("<h1>{}</h1>\n", title_esc));
    body.push_str(&format!(
        "<p class=\"meta\">Exported {} · {} messages</p>\n",
        html_escape(&meta.updated_at),
        meta.message_count
    ));

    for msg in messages {
        match msg.role {
            MessageRole::User => {
                if msg.content.trim().is_empty() {
                    continue;
                }
                body.push_str(&format!(
                    "<section class=\"msg user\"><h2>User</h2><div class=\"content\">{}</div></section>\n",
                    paragraphs(&msg.content)
                ));
            }
            MessageRole::Assistant | MessageRole::TextBlock => {
                let has_content = !msg.content.trim().is_empty();
                let thinking_text = if opts.include_thinking && msg.role == MessageRole::Assistant {
                    msg.thinking
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                } else {
                    None
                };
                if !has_content && thinking_text.is_none() {
                    continue;
                }
                body.push_str("<section class=\"msg assistant\"><h2>Assistant</h2>");
                if has_content {
                    body.push_str(&format!(
                        "<div class=\"content\">{}</div>",
                        paragraphs(&msg.content)
                    ));
                }
                if let Some(t) = thinking_text {
                    body.push_str(&format!(
                        "<details class=\"thinking\"><summary>Thinking</summary><div>{}</div></details>",
                        paragraphs(t)
                    ));
                }
                body.push_str("</section>\n");
            }
            MessageRole::ThinkingBlock => {
                if !opts.include_thinking || msg.content.trim().is_empty() {
                    continue;
                }
                body.push_str(&format!(
                    "<details class=\"thinking standalone\"><summary>Thinking</summary><div>{}</div></details>\n",
                    paragraphs(&msg.content)
                ));
            }
            MessageRole::Tool => {
                if !opts.include_tools {
                    continue;
                }
                let name = msg.tool_name.as_deref().unwrap_or("tool");
                body.push_str(&format!(
                    "<details class=\"tool\"><summary>Tool: {}</summary>",
                    html_escape(name)
                ));
                if let Some(args) = msg.tool_arguments.as_deref() {
                    body.push_str(&format!(
                        "<h4>Arguments</h4><pre><code>{}</code></pre>",
                        html_escape(args)
                    ));
                }
                if let Some(result) = msg.tool_result.as_deref() {
                    body.push_str(&format!(
                        "<h4>Result</h4><pre><code>{}</code></pre>",
                        html_escape(result)
                    ));
                }
                if let Some(meta) = msg.tool_metadata.as_deref() {
                    body.push_str(&format!(
                        "<h4>Metadata</h4><pre><code>{}</code></pre>",
                        html_escape(&pretty_json_or_raw(meta))
                    ));
                }
                body.push_str("</details>\n");
            }
            MessageRole::Event => {}
        }
    }

    let style = include_str!("export_style.css");
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title><style>{}</style></head><body><main>{}</main></body></html>",
        title_esc, style, body
    )
}

/// Wrap blank-line-separated chunks of text in `<p>` tags. Inside each
/// paragraph, line breaks become `<br>`. The whole input is HTML-escaped first
/// — no markdown / code-fence parsing here, since that would require a real
/// markdown engine and the export's HTML target is "open in any browser",
/// not "1:1 mirror of the in-app renderer".
fn paragraphs(s: &str) -> String {
    let escaped = html_escape(s);
    escaped
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!("<p>{}</p>", p.replace('\n', "<br>")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_meta() -> SessionMeta {
        SessionMeta {
            id: "sess-1".into(),
            title: Some("My Test Chat / 测试".into()),
            title_source: "manual".into(),
            agent_id: "ha-main".into(),
            provider_id: None,
            provider_name: None,
            model_id: None,
            reasoning_effort: None,
            created_at: "2026-05-10T10:00:00Z".into(),
            updated_at: "2026-05-10T10:30:00Z".into(),
            pinned_at: None,
            message_count: 4,
            unread_count: 0,
            channel_unread_count: 0,
            has_error: false,
            pending_interaction_count: 0,
            is_cron: false,
            parent_session_id: None,
            forked_from_session_id: None,
            forked_from_message_id: None,
            forked_from_session_title: None,
            plan_mode: Default::default(),
            execution_mode: Default::default(),
            workflow_mode: Default::default(),
            permission_mode: Default::default(),
            sandbox_mode: Default::default(),
            project_id: None,
            channel_info: None,
            incognito: false,
            working_dir: None,
            kind: Default::default(),
        }
    }

    fn fixture_messages() -> Vec<SessionMessage> {
        vec![
            SessionMessage {
                id: 1,
                session_id: "sess-1".into(),
                role: MessageRole::User,
                content: "Hello".into(),
                timestamp: "2026-05-10T10:00:00Z".into(),
                attachments_meta: None,
                model: None,
                tokens_in: None,
                tokens_out: None,
                reasoning_effort: None,
                tool_call_id: None,
                tool_name: None,
                tool_arguments: None,
                tool_result: None,
                tool_duration_ms: None,
                is_error: None,
                thinking: None,
                ttft_ms: None,
                tokens_in_last: None,
                tokens_cache_creation: None,
                tokens_cache_read: None,
                tool_metadata: None,
                stream_status: None,
            },
            SessionMessage {
                id: 2,
                session_id: "sess-1".into(),
                role: MessageRole::Assistant,
                content: "Hi there".into(),
                timestamp: "2026-05-10T10:01:00Z".into(),
                thinking: Some("internal CoT".into()),
                attachments_meta: None,
                model: None,
                tokens_in: None,
                tokens_out: None,
                reasoning_effort: None,
                tool_call_id: None,
                tool_name: None,
                tool_arguments: None,
                tool_result: None,
                tool_duration_ms: None,
                is_error: None,
                ttft_ms: None,
                tokens_in_last: None,
                tokens_cache_creation: None,
                tokens_cache_read: None,
                tool_metadata: None,
                stream_status: None,
            },
            SessionMessage {
                id: 3,
                session_id: "sess-1".into(),
                role: MessageRole::Tool,
                content: String::new(),
                timestamp: "2026-05-10T10:01:30Z".into(),
                tool_name: Some("read_file".into()),
                tool_arguments: Some("{\"path\":\"/tmp/x\"}".into()),
                tool_result: Some("file body here".into()),
                tool_duration_ms: Some(42),
                attachments_meta: None,
                model: None,
                tokens_in: None,
                tokens_out: None,
                reasoning_effort: None,
                tool_call_id: Some("call-abc".into()),
                is_error: None,
                thinking: None,
                ttft_ms: None,
                tokens_in_last: None,
                tokens_cache_creation: None,
                tokens_cache_read: None,
                tool_metadata: None,
                stream_status: None,
            },
            SessionMessage {
                id: 4,
                session_id: "sess-1".into(),
                role: MessageRole::Assistant,
                content: "Done".into(),
                timestamp: "2026-05-10T10:02:00Z".into(),
                attachments_meta: None,
                model: None,
                tokens_in: None,
                tokens_out: None,
                reasoning_effort: None,
                tool_call_id: None,
                tool_name: None,
                tool_arguments: None,
                tool_result: None,
                tool_duration_ms: None,
                is_error: None,
                thinking: None,
                ttft_ms: None,
                tokens_in_last: None,
                tokens_cache_creation: None,
                tokens_cache_read: None,
                tool_metadata: None,
                stream_status: None,
            },
        ]
    }

    #[test]
    fn markdown_lean_excludes_thinking_and_tool() {
        let opts = ExportOptions::lean(ExportFormat::Markdown);
        let md = to_markdown(&fixture_meta(), &fixture_messages(), opts);
        assert!(md.contains("# My Test Chat"));
        assert!(md.contains("## User\n\nHello"));
        assert!(md.contains("## Assistant\n\nHi there"));
        assert!(md.contains("## Assistant\n\nDone"));
        assert!(!md.contains("Thinking"));
        assert!(!md.contains("read_file"));
        assert!(!md.contains("internal CoT"));
    }

    #[test]
    fn markdown_full_includes_thinking_and_tool() {
        let opts = ExportOptions::full(ExportFormat::Markdown);
        let md = to_markdown(&fixture_meta(), &fixture_messages(), opts);
        assert!(md.contains("Thinking"));
        assert!(md.contains("internal CoT"));
        assert!(md.contains("Tool: read_file"));
        assert!(md.contains("**Arguments**"));
        assert!(md.contains("file body here"));
    }

    #[test]
    fn json_full_re_embeds_tool_arguments_object() {
        let opts = ExportOptions::full(ExportFormat::Json);
        let body = to_json(&fixture_meta(), &fixture_messages(), opts).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let messages = v["messages"].as_array().unwrap();
        let tool_msg = messages.iter().find(|m| m["role"] == "tool").unwrap();
        // Re-embedded as object, not a string.
        assert!(tool_msg["toolArguments"].is_object());
        assert_eq!(tool_msg["toolArguments"]["path"], "/tmp/x");
    }

    #[test]
    fn json_lean_drops_tool_and_thinking_fields() {
        let opts = ExportOptions::lean(ExportFormat::Json);
        let body = to_json(&fixture_meta(), &fixture_messages(), opts).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let messages = v["messages"].as_array().unwrap();
        // Tool messages are filtered entirely.
        assert!(messages.iter().all(|m| m["role"] != "tool"));
        // Thinking field stripped from assistant rows.
        let assistant = messages.iter().find(|m| m["role"] == "assistant").unwrap();
        assert!(assistant.get("thinking").is_none());
    }

    #[test]
    fn html_escapes_title_and_renders_paragraphs() {
        let opts = ExportOptions::full(ExportFormat::Html);
        let html = to_html(&fixture_meta(), &fixture_messages(), opts);
        assert!(html.starts_with("<!doctype html>"));
        // Title with `/` survives — it's not HTML-special.
        assert!(html.contains("My Test Chat / 测试"));
        assert!(html.contains("<section class=\"msg user\""));
        assert!(html.contains("<details class=\"tool\""));
        assert!(html.contains("Tool: read_file"));
    }

    #[test]
    fn html_escapes_dangerous_content() {
        let mut messages = fixture_messages();
        messages[0].content = "<script>alert(1)</script>".into();
        let html = to_html(
            &fixture_meta(),
            &messages,
            ExportOptions::lean(ExportFormat::Html),
        );
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn sanitize_filename_handles_specials_and_cjk() {
        assert_eq!(title_to_filename_stem("Hello / World"), "Hello _ World");
        assert_eq!(title_to_filename_stem("会议纪要"), "会议纪要");
        assert_eq!(title_to_filename_stem("   "), "session");
        assert_eq!(title_to_filename_stem("a:b*c?"), "a_b_c_");
    }

    #[test]
    fn export_format_parse_case_insensitive() {
        assert_eq!(ExportFormat::parse("md"), Some(ExportFormat::Markdown));
        assert_eq!(
            ExportFormat::parse("MARKDOWN"),
            Some(ExportFormat::Markdown)
        );
        assert_eq!(ExportFormat::parse(" Json "), Some(ExportFormat::Json));
        assert_eq!(ExportFormat::parse("HTML"), Some(ExportFormat::Html));
        assert_eq!(ExportFormat::parse("xml"), None);
    }

    #[test]
    fn tool_metadata_renders_in_markdown_and_html_when_tools_included() {
        let mut messages = fixture_messages();
        messages[2].tool_metadata =
            Some(r#"{"file":"/tmp/x","linesAdded":3,"linesRemoved":1}"#.into());

        let md = to_markdown(
            &fixture_meta(),
            &messages,
            ExportOptions::full(ExportFormat::Markdown),
        );
        assert!(md.contains("**Metadata**"));
        assert!(md.contains("linesAdded"));

        let html = to_html(
            &fixture_meta(),
            &messages,
            ExportOptions::full(ExportFormat::Html),
        );
        assert!(html.contains("<h4>Metadata</h4>"));
        assert!(html.contains("linesAdded"));

        // Lean export still omits it (tools fully filtered).
        let lean_md = to_markdown(
            &fixture_meta(),
            &messages,
            ExportOptions::lean(ExportFormat::Markdown),
        );
        assert!(!lean_md.contains("Metadata"));
    }

    #[test]
    fn html_keeps_thinking_only_assistant_messages() {
        let mut messages = fixture_messages();
        // Assistant row with no body but a non-empty `thinking` field — happens
        // when the model only reasons before invoking a tool.
        messages.push(SessionMessage {
            id: 5,
            session_id: "sess-1".into(),
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: "2026-05-10T10:03:00Z".into(),
            thinking: Some("reasoning without prose".into()),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: None,
            stream_status: None,
        });

        let html = to_html(
            &fixture_meta(),
            &messages,
            ExportOptions::full(ExportFormat::Html),
        );
        assert!(html.contains("reasoning without prose"));
    }
}
