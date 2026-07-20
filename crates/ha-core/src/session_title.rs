use anyhow::{anyhow, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

use crate::automation::{self, ModelTaskSpec};
use crate::provider::{ActiveModel, ModelChain};
use crate::session::SessionDB;

pub const TITLE_SOURCE_FIRST_MESSAGE: &str = "first_message";
pub const TITLE_SOURCE_LLM: &str = "llm";
pub const TITLE_SOURCE_MANUAL: &str = "manual";
const GOAL_TRIGGER_META_KEY: &str = "goal_trigger";
const LOOP_TRIGGER_META_KEY: &str = "loop_trigger";

static TITLE_GENERATION_IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTitleConfig {
    #[serde(default = "default_session_title_enabled")]
    pub enabled: bool,
    /// Deprecated — superseded by `modelOverride`. Kept for backward
    /// compatibility: still read when `modelOverride` is unset, but the GUI
    /// no longer writes these two fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Model chain override for title generation. `None` = fall through to
    /// the deprecated `provider_id`/`model_id` pair (if both set) →
    /// `function_models.automation` (title generation is exactly the kind
    /// of cheap, low-stakes background call that default is meant for) →
    /// the current chat's own model (a guaranteed final fallback, so title
    /// generation never fails outright even with zero config).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<ModelChain>,
}

fn default_session_title_enabled() -> bool {
    true
}

impl Default for SessionTitleConfig {
    fn default() -> Self {
        Self {
            enabled: default_session_title_enabled(),
            provider_id: None,
            model_id: None,
            model_override: None,
        }
    }
}

pub fn maybe_schedule_after_success(
    db: Arc<SessionDB>,
    session_id: String,
    agent_id: String,
    chat_model: ActiveModel,
) {
    maybe_schedule(db, session_id, agent_id, chat_model, true);
}

/// Autonomous turns can run for minutes before their first final assistant
/// row is written. Start title refinement from the durable user objective as
/// soon as Goal / Loop / Workflow execution begins; the normal success hook
/// retries with assistant context if this early attempt fails.
pub fn maybe_schedule_autonomous_start(
    db: Arc<SessionDB>,
    session_id: String,
    agent_id: String,
    chat_model: ActiveModel,
) {
    match is_autonomous_title_session(&db, &session_id) {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            app_warn!(
                "session",
                "title_generate",
                "Skipping autonomous title generation for session {}: {}",
                session_id,
                e
            );
            return;
        }
    }
    maybe_schedule(db, session_id, agent_id, chat_model, false);
}

fn maybe_schedule(
    db: Arc<SessionDB>,
    session_id: String,
    agent_id: String,
    chat_model: ActiveModel,
    require_assistant: bool,
) {
    let app_cfg = crate::config::cached_config();
    let cfg = app_cfg.session_title.clone();
    if !cfg.enabled {
        return;
    }

    let meta = match db.get_session(&session_id) {
        Ok(Some(meta)) => meta,
        Ok(None) => return,
        Err(e) => {
            app_warn!(
                "session",
                "title_generate",
                "Skipping title generation: failed to load session {}: {}",
                session_id,
                e
            );
            return;
        }
    };

    if meta.incognito {
        return;
    }
    if meta.title_source != TITLE_SOURCE_FIRST_MESSAGE {
        let repaired = meta.title_source == TITLE_SOURCE_MANUAL
            && repair_misclassified_goal_fallback(&db, &meta).unwrap_or_else(|e| {
                app_warn!(
                    "session",
                    "title_generate",
                    "Failed to inspect legacy Goal title for session {}: {}",
                    session_id,
                    e
                );
                false
            });
        if !repaired {
            return;
        }
        app_info!(
            "session",
            "title_generate",
            "Recovered auto-generated Goal title source for session {}",
            session_id
        );
    }

    // Candidate chain, in try-order: `model_override` (new) → deprecated
    // `provider_id`/`model_id` pair → `function_models.automation` (title
    // generation is a cheap, low-stakes background call — exactly what that
    // default is meant for) → the current chat's own model (a guaranteed
    // final fallback, so title generation never fails outright even with
    // zero config).
    let legacy_chain = cfg
        .provider_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .zip(cfg.model_id.as_deref().filter(|s| !s.trim().is_empty()))
        .map(|(provider_id, model_id)| ModelChain {
            primary: ActiveModel {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
            },
            fallbacks: Vec::new(),
        });
    let mut chain = cfg
        .model_override
        .clone()
        .or(legacy_chain)
        .or_else(|| app_cfg.function_models.automation.clone())
        .map(ModelChain::into_vec)
        .unwrap_or_default();
    chain.push(chat_model);

    if !claim_title_generation(&session_id) {
        return;
    }
    let lease = TitleGenerationLease(session_id.clone());
    let eval_model_guard = match crate::eval_context::retain_model_automation(&session_id) {
        Ok(guard) => guard,
        Err(error) => {
            app_warn!(
                "session",
                "title_generate",
                "Skipping evaluation title generation at its immutable budget: {}",
                error
            );
            return;
        }
    };

    std::thread::spawn(move || {
        let _lease = lease;
        let _eval_model_guard = eval_model_guard;
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                app_warn!(
                    "session",
                    "title_generate",
                    "Failed to create title generation runtime: {}",
                    e
                );
                return;
            }
        };

        if let Err(e) = rt.block_on(generate_and_update_title(
            db,
            session_id,
            agent_id,
            chain,
            require_assistant,
        )) {
            app_warn!(
                "session",
                "title_generate",
                "Title generation failed: {}",
                e
            );
        }
    });
}

fn claim_title_generation(session_id: &str) -> bool {
    TITLE_GENERATION_IN_FLIGHT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(session_id.to_string())
}

struct TitleGenerationLease(String);

impl Drop for TitleGenerationLease {
    fn drop(&mut self) {
        TITLE_GENERATION_IN_FLIGHT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.0);
    }
}

fn is_autonomous_title_session(db: &SessionDB, session_id: &str) -> Result<bool> {
    let conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
    let workflow_mode = conn
        .query_row(
            "SELECT workflow_mode FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if workflow_mode
        .as_deref()
        .is_some_and(|mode| !mode.trim().is_empty() && mode != "off")
    {
        return Ok(true);
    }

    let mut stmt = conn.prepare(
        "SELECT role, attachments_meta
           FROM messages
          WHERE session_id = ?1 AND attachments_meta IS NOT NULL
          ORDER BY id ASC
          LIMIT 32",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (role, raw_meta) = row?;
        let Some(meta) = parse_attachments_meta(Some(&raw_meta)) else {
            continue;
        };
        if role == "user"
            && (meta
                .get(GOAL_TRIGGER_META_KEY)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                || meta.get(LOOP_TRIGGER_META_KEY).is_some())
        {
            return Ok(true);
        }
        if role == "event" && is_autonomous_slash_command(&meta) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn repair_misclassified_goal_fallback(
    db: &SessionDB,
    meta: &crate::session::SessionMeta,
) -> Result<bool> {
    let Some(current_title) = meta.title.as_deref() else {
        return Ok(false);
    };
    let (content, attachments_meta, user_count) = {
        let conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        let user_count = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = 'user'",
            rusqlite::params![meta.id],
            |row| row.get::<_, i64>(0),
        )?;
        let first = conn
            .query_row(
                "SELECT content, attachments_meta
                   FROM messages
                  WHERE session_id = ?1 AND role = 'user'
                  ORDER BY id ASC
                  LIMIT 1",
                rusqlite::params![meta.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let Some((content, attachments_meta)) = first else {
            return Ok(false);
        };
        (content, attachments_meta, user_count)
    };

    // This recovery is intentionally limited to the old malformed Goal path
    // that persisted the literal command prefix. Normal manual titles remain
    // authoritative even when they happen to resemble the first message.
    if user_count != 1
        || !content
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("/goal ")
        || !is_goal_trigger(attachments_meta.as_deref())
    {
        return Ok(false);
    }
    let expected = crate::session::first_message_title_candidate(
        &meta.id,
        &content,
        attachments_meta.as_deref(),
    );
    if expected.as_deref() != Some(current_title) {
        return Ok(false);
    }

    db.update_session_title_source_if_title_and_source(
        &meta.id,
        current_title,
        TITLE_SOURCE_MANUAL,
        TITLE_SOURCE_FIRST_MESSAGE,
    )
}

fn is_goal_trigger(attachments_meta: Option<&str>) -> bool {
    parse_attachments_meta(attachments_meta)
        .and_then(|value| value.get(GOAL_TRIGGER_META_KEY).cloned())
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn parse_attachments_meta(raw: Option<&str>) -> Option<serde_json::Value> {
    raw.and_then(|value| serde_json::from_str(value).ok())
}

fn is_autonomous_slash_command(meta: &serde_json::Value) -> bool {
    let Some(command) = meta.get("slash_command") else {
        return false;
    };
    command.get("kind").and_then(serde_json::Value::as_str) == Some("command")
        && command.get("displayAs").and_then(serde_json::Value::as_str) == Some("user")
        && matches!(
            command.get("mode").and_then(serde_json::Value::as_str),
            Some("goal") | Some("loop")
        )
}

async fn generate_and_update_title(
    db: Arc<SessionDB>,
    session_id: String,
    _agent_id: String,
    chain: Vec<ActiveModel>,
    require_assistant: bool,
) -> Result<()> {
    let messages = collect_title_messages(&db, &session_id)?;
    let user_count = messages
        .iter()
        .filter(|line| line.starts_with("User:"))
        .count();
    let has_assistant = messages.iter().any(|line| line.starts_with("Assistant:"));
    if user_count != 1 || (require_assistant && !has_assistant) {
        return Ok(());
    }

    let prompt = build_title_prompt(&messages);
    let response = automation::run(ModelTaskSpec {
        purpose: "session_title",
        chain,
        session_key: &session_id,
        instruction: &prompt,
        max_tokens: 64,
    })
    .await
    .map_err(|e| {
        anyhow!(
            "session title generation failed (session={}): {}",
            session_id,
            e
        )
    })?
    .text;

    let title = sanitize_generated_title(&response)
        .ok_or_else(|| anyhow!("session title model returned an empty title"))?;

    let updated = db.update_session_title_if_source(
        &session_id,
        TITLE_SOURCE_FIRST_MESSAGE,
        &title,
        TITLE_SOURCE_LLM,
    )?;
    if updated {
        app_info!(
            "session",
            "title_generate",
            "Generated LLM title for session {}",
            session_id
        );
        if let Some(bus) = crate::get_event_bus() {
            bus.emit(
                "session:title_updated",
                serde_json::json!({
                    "sessionId": session_id,
                    "title": title,
                }),
            );
        }
    }

    Ok(())
}

fn collect_title_messages(db: &SessionDB, session_id: &str) -> Result<Vec<String>> {
    let conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT role, content, attachments_meta
             FROM messages
             WHERE session_id = ?1 AND role IN ('user', 'assistant', 'event')
             ORDER BY id ASC
             LIMIT 32",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;

    let mut lines = Vec::new();
    let mut saw_user = false;
    for row in rows {
        let (role, content, attachments_meta) = row?;
        let label = match role.as_str() {
            "user" if !saw_user => {
                saw_user = true;
                "User"
            }
            "event"
                if !saw_user
                    && parse_attachments_meta(attachments_meta.as_deref())
                        .as_ref()
                        .is_some_and(is_autonomous_slash_command) =>
            {
                saw_user = true;
                "User"
            }
            "assistant" if saw_user => "Assistant",
            _ => continue,
        };
        let content = content.trim();
        if content.is_empty() {
            continue;
        }
        let content = crate::truncate_utf8(content, 2000).replace('\n', " ");
        lines.push(format!("{label}: {content}"));
        if label == "Assistant" {
            break;
        }
    }
    Ok(lines)
}

fn build_title_prompt(messages: &[String]) -> String {
    format!(
        "Generate a concise chat session title from the conversation below.\n\
         Rules:\n\
         - Return only the title text, no quotes, no markdown, no explanation.\n\
         - Use the same language as the conversation when clear.\n\
         - Keep it short and specific, ideally 3-8 words.\n\
         - Do not include private credentials or long identifiers.\n\n\
         Conversation:\n{}",
        messages.join("\n")
    )
}

pub fn sanitize_generated_title(raw: &str) -> Option<String> {
    let mut line = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_string();

    line = trim_title_quotes(&line)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    for prefix in ["Title:", "title:", "标题:", "标题："] {
        if let Some(rest) = line.strip_prefix(prefix) {
            line = rest.trim().to_string();
            break;
        }
    }
    line = trim_title_quotes(&line);

    line = line
        .trim_matches(|c: char| matches!(c, '.' | '。' | ':' | '：'))
        .trim()
        .to_string();

    if line.is_empty() {
        None
    } else {
        Some(crate::session::auto_title(&line))
    }
}

fn trim_title_quotes(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | '“' | '”' | '‘' | '’' | '「' | '」' | '『' | '』'
            )
        })
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_title_config_defaults_enabled() {
        let cfg = SessionTitleConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.provider_id.is_none());
        assert!(cfg.model_id.is_none());
    }

    #[test]
    fn session_title_config_deserializes_missing_enabled_as_enabled() {
        let cfg: SessionTitleConfig = serde_json::from_value(serde_json::json!({}))
            .expect("deserialize empty session title config");

        assert!(cfg.enabled);
        assert!(cfg.provider_id.is_none());
        assert!(cfg.model_id.is_none());
    }

    #[test]
    fn sanitize_generated_title_strips_noise() {
        assert_eq!(
            sanitize_generated_title("\"Title: Rust 错误排查。\"").as_deref(),
            Some("Rust 错误排查")
        );
        assert_eq!(
            sanitize_generated_title("Title: \"Rust 错误排查。\"").as_deref(),
            Some("Rust 错误排查")
        );
        assert_eq!(sanitize_generated_title("\n\n").as_deref(), None);
    }

    #[test]
    fn repairs_only_exact_goal_fallback_titles_misclassified_as_manual() {
        let db_path = std::env::temp_dir().join(format!(
            "hope-agent-session-title-repair-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = std::sync::Arc::new(SessionDB::open(&db_path).expect("open session db"));
        crate::channel::ChannelDB::new(db.clone())
            .migrate()
            .expect("migrate channel schema");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        let content = "/goal 完成跨平台发布检查并输出报告";
        let mut user = crate::session::NewMessage::user(content);
        user.attachments_meta = Some(serde_json::json!({ "goal_trigger": true }).to_string());
        db.append_message(&session.id, &user)
            .expect("append goal message");
        let fallback = crate::session::auto_title(content);
        db.update_session_title(&session.id, &fallback)
            .expect("simulate legacy fallback");

        let meta = db
            .get_session(&session.id)
            .expect("load session")
            .expect("session exists");
        assert_eq!(meta.title_source, TITLE_SOURCE_MANUAL);
        assert!(repair_misclassified_goal_fallback(&db, &meta).expect("repair fallback"));
        assert_eq!(
            db.get_session(&session.id)
                .expect("reload session")
                .expect("session exists")
                .title_source,
            TITLE_SOURCE_FIRST_MESSAGE
        );

        db.update_session_title(&session.id, "用户自定义标题")
            .expect("manual rename");
        let manual = db
            .get_session(&session.id)
            .expect("load manual session")
            .expect("session exists");
        assert!(!repair_misclassified_goal_fallback(&db, &manual).expect("preserve manual title"));
        assert_eq!(
            db.get_session(&session.id)
                .expect("reload manual session")
                .expect("session exists")
                .title
                .as_deref(),
            Some("用户自定义标题")
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    }

    #[test]
    fn autonomous_title_detection_covers_goal_loop_and_workflow() {
        let db_path = std::env::temp_dir().join(format!(
            "hope-agent-autonomous-title-detection-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = SessionDB::open(&db_path).expect("open session db");

        let goal = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create goal session");
        let mut goal_message = crate::session::NewMessage::user("完成发布检查");
        goal_message.attachments_meta =
            Some(serde_json::json!({ "goal_trigger": true }).to_string());
        db.append_message(&goal.id, &goal_message)
            .expect("append goal message");
        assert!(is_autonomous_title_session(&db, &goal.id).expect("detect goal"));

        let loop_session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create loop session");
        let mut loop_event = crate::session::NewMessage::event("每小时检查发布状态");
        loop_event.attachments_meta = Some(
            serde_json::json!({
                "slash_command": {
                    "kind": "command",
                    "command": "/loop 每小时检查发布状态",
                    "displayAs": "user",
                    "mode": "loop"
                }
            })
            .to_string(),
        );
        db.append_message(&loop_session.id, &loop_event)
            .expect("append loop command");
        assert!(is_autonomous_title_session(&db, &loop_session.id).expect("detect loop"));

        let workflow = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create workflow session");
        db.update_session_workflow_mode(&workflow.id, crate::workflow_mode::WorkflowMode::On)
            .expect("enable workflow mode");
        assert!(is_autonomous_title_session(&db, &workflow.id).expect("detect workflow"));

        let plain = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create plain session");
        assert!(!is_autonomous_title_session(&db, &plain.id).expect("detect plain chat"));

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    }

    #[test]
    fn collect_title_messages_prefers_visible_loop_command_over_internal_trigger() {
        let db_path = std::env::temp_dir().join(format!(
            "hope-agent-loop-title-context-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = SessionDB::open(&db_path).expect("open session db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        let mut command = crate::session::NewMessage::event("每小时检查发布状态");
        command.attachments_meta = Some(
            serde_json::json!({
                "slash_command": {
                    "kind": "command",
                    "command": "/loop 每小时检查发布状态",
                    "displayAs": "user",
                    "mode": "loop"
                }
            })
            .to_string(),
        );
        db.append_message(&session.id, &command)
            .expect("append visible command");
        let mut trigger = crate::session::NewMessage::user(
            "<loop_trigger><loop_id>internal</loop_id></loop_trigger>",
        );
        trigger.attachments_meta =
            Some(serde_json::json!({ "loop_trigger": { "run_id": "run-1" } }).to_string());
        db.append_message(&session.id, &trigger)
            .expect("append internal trigger");
        db.append_message(
            &session.id,
            &crate::session::NewMessage::assistant("本轮检查完成"),
        )
        .expect("append assistant");

        assert_eq!(
            collect_title_messages(&db, &session.id).expect("collect title context"),
            vec!["User: 每小时检查发布状态", "Assistant: 本轮检查完成"]
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    }

    #[test]
    fn collect_title_messages_uses_first_turn_when_later_turn_exists() {
        let db_path = std::env::temp_dir().join(format!(
            "hope-agent-session-title-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = SessionDB::open(&db_path).expect("open session db");
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session");
        db.append_message(
            &session.id,
            &crate::session::NewMessage::user("第一条用户消息"),
        )
        .expect("append first user");
        db.append_message(
            &session.id,
            &crate::session::NewMessage::assistant("第一条助手回复"),
        )
        .expect("append first assistant");
        db.append_message(
            &session.id,
            &crate::session::NewMessage::user("第二条用户追问"),
        )
        .expect("append second user");

        let lines = collect_title_messages(&db, &session.id).expect("collect title messages");
        assert_eq!(
            lines,
            vec!["User: 第一条用户消息", "Assistant: 第一条助手回复"]
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    }
}
