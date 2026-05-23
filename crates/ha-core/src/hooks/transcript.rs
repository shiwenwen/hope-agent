//! JSONL transcript mirror (design doc §10).
//!
//! `transcript_path` in the hook input points at
//! `~/.hope-agent/sessions/{id}/transcript.jsonl`, a line-per-message mirror of
//! the session so official hook scripts can `jq` it. Phase 0.1 ships the write
//! primitive + a best-effort backfill of existing sessions; wiring the
//! per-message synchronous append rides along with the event embeds in a later
//! commit.
//!
//! The mapping from the flat [`SessionMessage`] to the official block-shaped
//! transcript line is intentionally lossy (design §10.4 allows drift) — it only
//! needs each line to be valid JSON with a `type` and a `message`.

use std::fs;
use std::io::Write as _;

use serde::Serialize;

use crate::session::{MessageRole, SessionDB, SessionMessage};

/// One transcript JSONL line.
#[derive(Debug, Clone, Serialize)]
pub struct TranscriptLine {
    #[serde(rename = "type")]
    pub line_type: String,
    pub message: TranscriptMessage,
    pub timestamp: String,
    pub uuid: String,
    #[serde(rename = "parentUuid", skip_serializing_if = "Option::is_none")]
    pub parent_uuid: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
    pub version: String,
}

/// The `message` object inside a transcript line.
#[derive(Debug, Clone, Serialize)]
pub struct TranscriptMessage {
    pub role: String,
    pub content: Vec<serde_json::Value>,
}

fn text_block(text: &str) -> serde_json::Value {
    serde_json::json!({ "type": "text", "text": text })
}

/// Map one flat [`SessionMessage`] to a transcript line. Returns `None` only
/// for rows that carry no useful content.
pub fn session_message_to_line(
    msg: &SessionMessage,
    cwd: &str,
    parent_uuid: Option<String>,
) -> Option<TranscriptLine> {
    let (line_type, role, content) = match msg.role {
        MessageRole::User => ("user", "user", vec![text_block(&msg.content)]),
        MessageRole::Assistant | MessageRole::TextBlock | MessageRole::ThinkingBlock => {
            ("assistant", "assistant", vec![text_block(&msg.content)])
        }
        MessageRole::Tool => {
            let block = serde_json::json!({
                "type": "tool_result",
                "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                "content": msg.tool_result.clone().unwrap_or_default(),
                "is_error": msg.is_error.unwrap_or(false),
            });
            ("tool_result", "tool", vec![block])
        }
        MessageRole::Event => ("system", "system", vec![text_block(&msg.content)]),
    };

    Some(TranscriptLine {
        line_type: line_type.to_string(),
        message: TranscriptMessage {
            role: role.to_string(),
            content,
        },
        timestamp: msg.timestamp.clone(),
        uuid: format!("msg_{}", msg.id),
        parent_uuid,
        session_id: msg.session_id.clone(),
        cwd: cwd.to_string(),
        version: "1".to_string(),
    })
}

/// Render an ordered message slice to a JSONL string (one line per message,
/// parent-linked). Pure — the unit-test surface for the mapping.
pub fn messages_to_jsonl(messages: &[SessionMessage], cwd: &str) -> String {
    let mut out = String::new();
    let mut parent: Option<String> = None;
    for m in messages {
        if let Some(line) = session_message_to_line(m, cwd, parent.clone()) {
            // serde of a plain struct of String/Value never fails.
            if let Ok(json) = serde_json::to_string(&line) {
                out.push_str(&json);
                out.push('\n');
            }
            parent = Some(line.uuid);
        }
    }
    out
}

/// Namespace for the transcript mirror operations.
pub struct TranscriptMirror;

impl TranscriptMirror {
    /// Append one line to a session's transcript, creating the directory and
    /// file on first write.
    pub fn append_line(session_id: &str, line: &TranscriptLine) -> anyhow::Result<()> {
        let dir = crate::paths::session_dir(session_id)?;
        fs::create_dir_all(&dir)?;
        let path = dir.join("transcript.jsonl");
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let json = serde_json::to_string(line)?;
        writeln!(f, "{json}")?;
        Ok(())
    }

    /// Backfill a single session's transcript from the DB if it doesn't exist
    /// yet. Returns `true` when a file was written.
    pub fn backfill_session(db: &SessionDB, session_id: &str) -> anyhow::Result<bool> {
        let dir = crate::paths::session_dir(session_id)?;
        let path = dir.join("transcript.jsonl");
        if path.exists() {
            return Ok(false);
        }
        let messages = db.load_session_messages(session_id)?;
        if messages.is_empty() {
            return Ok(false);
        }
        // Phase 0.1: cwd left empty for the historical backfill (the hook input
        // carries the live cwd; transcript cwd is best-effort, design §10.4).
        let jsonl = messages_to_jsonl(&messages, "");
        fs::create_dir_all(&dir)?;
        fs::write(&path, jsonl)?;
        Ok(true)
    }

    /// Backfill every session missing a transcript. Best-effort: a failing
    /// session is logged and skipped. Returns the number written.
    pub fn backfill_all(db: &SessionDB) -> anyhow::Result<usize> {
        let sessions = db.list_sessions(None)?;
        let mut written = 0usize;
        for meta in sessions {
            match Self::backfill_session(db, &meta.id) {
                Ok(true) => written += 1,
                Ok(false) => {}
                Err(e) => {
                    app_warn!(
                        "hooks",
                        "transcript",
                        "backfill skipped session {}: {}",
                        meta.id,
                        e
                    );
                }
            }
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: i64, role: MessageRole, content: &str) -> SessionMessage {
        SessionMessage {
            id,
            session_id: "sess1".into(),
            role,
            content: content.into(),
            timestamp: "2026-05-22T00:00:00+00:00".into(),
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
        }
    }

    #[test]
    fn user_message_maps_to_user_text() {
        let line =
            session_message_to_line(&msg(1, MessageRole::User, "hello"), "/w", None).unwrap();
        assert_eq!(line.line_type, "user");
        assert_eq!(line.message.role, "user");
        assert_eq!(line.message.content[0]["type"], "text");
        assert_eq!(line.message.content[0]["text"], "hello");
        assert_eq!(line.uuid, "msg_1");
        assert_eq!(line.cwd, "/w");
        assert_eq!(line.version, "1");
    }

    #[test]
    fn assistant_variants_map_to_assistant() {
        for role in [
            MessageRole::Assistant,
            MessageRole::TextBlock,
            MessageRole::ThinkingBlock,
        ] {
            let line = session_message_to_line(&msg(2, role, "x"), "", None).unwrap();
            assert_eq!(line.line_type, "assistant");
            assert_eq!(line.message.role, "assistant");
        }
    }

    #[test]
    fn tool_row_maps_to_tool_result_block() {
        let mut m = msg(3, MessageRole::Tool, "");
        m.tool_call_id = Some("call_9".into());
        m.tool_result = Some("output".into());
        m.is_error = Some(true);
        let line = session_message_to_line(&m, "", None).unwrap();
        assert_eq!(line.line_type, "tool_result");
        let block = &line.message.content[0];
        assert_eq!(block["type"], "tool_result");
        assert_eq!(block["tool_use_id"], "call_9");
        assert_eq!(block["content"], "output");
        assert_eq!(block["is_error"], true);
    }

    #[test]
    fn jsonl_is_parent_linked_and_one_line_each() {
        let msgs = vec![
            msg(1, MessageRole::User, "q"),
            msg(2, MessageRole::Assistant, "a"),
        ];
        let jsonl = messages_to_jsonl(&msgs, "/w");
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        // Each line is valid JSON.
        let l0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        let l1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(l0["type"], "user");
        assert_eq!(l0["sessionId"], "sess1");
        assert!(l0.get("parentUuid").is_none()); // first line has no parent
                                                 // Second line's parentUuid links to the first line's uuid.
        assert_eq!(l1["parentUuid"], l0["uuid"]);
        assert_eq!(l1["uuid"], "msg_2");
    }
}
