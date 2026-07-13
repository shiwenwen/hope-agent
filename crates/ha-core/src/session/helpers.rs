use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use super::types::SessionMeta;
use crate::provider::ActiveModel;
use serde::{Deserialize, Serialize};

/// Fully resolved chat defaults for a draft or materialized Session. The
/// preferred model is retained even while its Provider is disabled; `model`
/// is the first currently usable entry in the effective chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRuntimeDefaults {
    pub preferred_model: Option<ActiveModel>,
    pub model: Option<ActiveModel>,
    pub preferred_model_available: bool,
    pub temperature: Option<f64>,
    pub reasoning_effort: String,
}

pub fn resolve_chat_runtime_defaults(
    session: Option<&SessionMeta>,
    agent_id: &str,
) -> ChatRuntimeDefaults {
    let config = crate::config::cached_config();
    let agent_model = crate::agent_loader::load_agent(agent_id)
        .ok()
        .map(|definition| definition.config.model)
        .unwrap_or_default();
    let preferred_model = session.and_then(|meta| match (&meta.provider_id, &meta.model_id) {
        (Some(provider_id), Some(model_id)) if !provider_id.is_empty() && !model_id.is_empty() => {
            Some(ActiveModel {
                provider_id: provider_id.clone(),
                model_id: model_id.clone(),
            })
        }
        _ => None,
    });
    let preferred_ref = preferred_model
        .as_ref()
        .map(|model| format!("{}::{}", model.provider_id, model.model_id));
    let initialized = session.is_some_and(|meta| meta.runtime_defaults_initialized);
    // An initialized Session with no preferred model represents a real
    // "unconfigured" snapshot (for example after the last usable model was
    // deleted). Do not silently start inheriting a model added later.
    let model = if initialized && preferred_model.is_none() {
        None
    } else {
        crate::provider::resolve_model_chain_with_preferred(
            preferred_ref.as_deref(),
            &agent_model,
            &config,
        )
        .0
    };
    let temperature = if initialized {
        session.and_then(|meta| meta.temperature)
    } else {
        agent_model.temperature.or(config.temperature)
    };
    let reasoning_effort = if initialized {
        session
            .and_then(|meta| meta.reasoning_effort.clone())
            .unwrap_or_else(|| config.reasoning_effort.clone())
    } else {
        agent_model
            .reasoning_effort
            .clone()
            .unwrap_or_else(|| config.reasoning_effort.clone())
    };
    let preferred_model_available = preferred_model.as_ref().is_none_or(|preferred| {
        crate::provider::model_ref_is_available(&config.providers, preferred)
    });
    ChatRuntimeDefaults {
        preferred_model,
        model,
        preferred_model_available,
        temperature,
        reasoning_effort,
    }
}

/// Upgrade a legacy Session exactly once, preserving any existing model/Think
/// values and snapshotting the missing temperature (including `None`).
pub fn ensure_session_runtime_defaults(
    db: &super::SessionDB,
    session_id: &str,
) -> Result<ChatRuntimeDefaults> {
    let meta = db
        .get_session(session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
    if !meta.runtime_defaults_initialized {
        let defaults = resolve_chat_runtime_defaults(Some(&meta), &meta.agent_id);
        let provider_name = defaults.model.as_ref().and_then(|model| {
            crate::config::cached_config()
                .providers
                .iter()
                .find(|provider| provider.id == model.provider_id)
                .map(|provider| provider.name.clone())
        });
        db.initialize_session_runtime_defaults(
            session_id,
            defaults
                .model
                .as_ref()
                .map(|model| model.provider_id.as_str()),
            provider_name.as_deref(),
            defaults.model.as_ref().map(|model| model.model_id.as_str()),
            defaults.temperature,
            &defaults.reasoning_effort,
        )?;
    }
    let refreshed = db
        .get_session(session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
    Ok(resolve_chat_runtime_defaults(
        Some(&refreshed),
        &refreshed.agent_id,
    ))
}

pub fn set_session_model_preference(
    db: &super::SessionDB,
    session_id: &str,
    provider_id: &str,
    model_id: &str,
) -> Result<()> {
    if db.get_session(session_id)?.is_none() {
        anyhow::bail!("session not found: {session_id}");
    }
    let config = crate::config::cached_config();
    let model = ActiveModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    };
    if !crate::provider::model_ref_exists(&config.providers, &model) {
        anyhow::bail!("Selected model no longer exists: {provider_id}::{model_id}");
    }
    let provider_name = config
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .map(|provider| provider.name.as_str());
    db.update_session_model(session_id, Some(provider_id), provider_name, Some(model_id))
}

pub fn set_session_temperature_preference(
    db: &super::SessionDB,
    session_id: &str,
    value: Option<f64>,
    use_agent_default: bool,
) -> Result<Option<f64>> {
    if db.get_session(session_id)?.is_none() {
        anyhow::bail!("session not found: {session_id}");
    }
    let temperature = if use_agent_default {
        let meta = db
            .get_session(session_id)?
            .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
        let agent_temperature = crate::agent_loader::load_agent(&meta.agent_id)
            .ok()
            .and_then(|definition| definition.config.model.temperature);
        agent_temperature.or(crate::config::cached_config().temperature)
    } else {
        let value = value.ok_or_else(|| anyhow::anyhow!("temperature value is required"))?;
        if !(0.0..=2.0).contains(&value) {
            anyhow::bail!("Temperature must be between 0.0 and 2.0");
        }
        Some(value)
    };
    db.update_session_temperature(session_id, temperature)?;
    Ok(temperature)
}

pub fn set_session_reasoning_effort_preference(
    db: &super::SessionDB,
    session_id: &str,
    value: Option<&str>,
    use_agent_default: bool,
) -> Result<String> {
    if db.get_session(session_id)?.is_none() {
        anyhow::bail!("session not found: {session_id}");
    }
    let effort = if use_agent_default {
        let meta = db
            .get_session(session_id)?
            .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
        crate::agent_loader::load_agent(&meta.agent_id)
            .ok()
            .and_then(|definition| definition.config.model.reasoning_effort)
            .unwrap_or_else(|| crate::config::cached_config().reasoning_effort.clone())
    } else {
        value
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("reasoning effort value is required"))?
    };
    if !crate::agent::is_valid_reasoning_effort(&effort) {
        anyhow::bail!("Invalid reasoning effort: {effort}");
    }
    db.update_session_reasoning_effort(session_id, Some(&effort))?;
    Ok(effort)
}

// ── Auto-title helper ────────────────────────────────────────────

/// Generate a short title from the first user message (truncated to 50 chars).
pub fn auto_title(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return "New Chat".to_string();
    }
    // Take first line only
    let first_line = trimmed.lines().next().unwrap_or(trimmed);
    // Use char count (not byte length) to handle CJK/emoji correctly
    if first_line.chars().count() <= 50 {
        first_line.to_string()
    } else {
        // Find the byte offset of the 47th character boundary
        let cut = first_line
            .char_indices()
            .nth(47)
            .map(|(i, _)| i)
            .unwrap_or(first_line.len());
        format!("{}...", &first_line[..cut])
    }
}

fn user_attachment_entries(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map
            .get("user_attachments")
            .and_then(Value::as_array)
            .map(|items| items.iter().collect())
            .unwrap_or_else(|| vec![value]),
        _ => Vec::new(),
    }
}

fn safe_pasted_text_first_line(session_id: &str, attachment: &Value) -> Option<String> {
    if attachment.get("source").and_then(Value::as_str)
        != Some(crate::attachments::PASTED_TEXT_SOURCE)
    {
        return None;
    }
    let raw_path = attachment.get("path").and_then(Value::as_str)?;
    let allowed_dir = crate::paths::attachments_dir(session_id)
        .ok()?
        .canonicalize()
        .ok()?;
    let path = Path::new(raw_path).canonicalize().ok()?;
    if !path.starts_with(&allowed_dir) {
        return None;
    }

    let reader = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    for line in reader.lines().take(16) {
        let line = line.ok()?;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn message_quote_first_line(attachment: &Value) -> Option<String> {
    if attachment.get("kind").and_then(Value::as_str)
        != Some(crate::attachments::MESSAGE_QUOTE_SOURCE)
    {
        return None;
    }
    attachment
        .get("content")
        .and_then(Value::as_str)?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Build a non-empty fallback title from the first user-visible input.
///
/// Large pasted text is persisted as an attachment and leaves the message body
/// empty. In that case, prefer the first readable line from the persisted file,
/// then fall back to the attachment name. The persisted path is only read after
/// canonicalizing it beneath this session's attachment directory.
pub fn first_message_title_candidate(
    session_id: &str,
    content: &str,
    attachments_meta: Option<&str>,
) -> Option<String> {
    if !content.trim().is_empty() {
        return Some(auto_title(content));
    }

    let parsed = attachments_meta.and_then(|raw| serde_json::from_str::<Value>(raw).ok())?;
    let attachments = user_attachment_entries(&parsed);
    for attachment in &attachments {
        if let Some(first_line) = message_quote_first_line(attachment) {
            return Some(auto_title(&first_line));
        }
    }
    for attachment in &attachments {
        if let Some(first_line) = safe_pasted_text_first_line(session_id, attachment) {
            return Some(auto_title(&first_line));
        }
    }
    for attachment in attachments {
        let Some(name) = attachment.get("name").and_then(Value::as_str) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let stem = Path::new(name)
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(name);
        return Some(auto_title(stem));
    }
    None
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod title_tests {
    use super::first_message_title_candidate;

    #[test]
    fn title_candidate_uses_pasted_text_first_line_before_file_name() {
        let session_id = format!("title-candidate-{}", uuid::Uuid::new_v4());
        let dir = crate::paths::attachments_dir(&session_id).expect("attachment dir");
        std::fs::create_dir_all(&dir).expect("create attachment dir");
        let path = dir.join("long-paste.txt");
        std::fs::write(&path, "\n这是粘贴内容的第一行\n第二行").expect("write pasted text");
        let meta = serde_json::json!([{
            "name": "long-paste.txt",
            "path": path,
            "source": crate::attachments::PASTED_TEXT_SOURCE,
        }])
        .to_string();

        assert_eq!(
            first_message_title_candidate(&session_id, "", Some(&meta)).as_deref(),
            Some("这是粘贴内容的第一行")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn title_candidate_falls_back_to_nested_attachment_name() {
        let meta = serde_json::json!({
            "goal_trigger": true,
            "user_attachments": [{
                "name": "产品调研记录.txt",
                "source": crate::attachments::PASTED_TEXT_SOURCE,
            }]
        })
        .to_string();

        assert_eq!(
            first_message_title_candidate("missing-session", "", Some(&meta)).as_deref(),
            Some("产品调研记录")
        );
        assert_eq!(
            first_message_title_candidate("missing-session", "", None),
            None
        );
    }

    #[test]
    fn title_candidate_uses_message_quote_content() {
        let meta = serde_json::json!([{
            "kind": crate::attachments::MESSAGE_QUOTE_SOURCE,
            "role": "assistant",
            "content": "\nQuoted conversation line\nsecond line",
        }])
        .to_string();

        assert_eq!(
            first_message_title_candidate("session-a", "", Some(&meta)).as_deref(),
            Some("Quoted conversation line")
        );
    }
}

/// Set the immediate fallback title from the first user-visible message.
/// Returns the title when a write happened.
pub fn ensure_first_message_title(
    db: &super::SessionDB,
    session_id: &str,
    content: &str,
    attachments_meta: Option<&str>,
) -> Result<Option<String>> {
    let should_update = {
        let conn = db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let Some((title, incognito, message_count)) = conn
            .query_row(
                "SELECT s.title, s.incognito,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) AS message_count
                   FROM sessions s
                  WHERE s.id = ?1",
                rusqlite::params![session_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        !incognito && title.is_none() && message_count <= 1
    };

    if should_update {
        let Some(title) = first_message_title_candidate(session_id, content, attachments_meta)
        else {
            return Ok(None);
        };
        db.update_session_title_with_source(
            session_id,
            &title,
            crate::session_title::TITLE_SOURCE_FIRST_MESSAGE,
        )?;
        if let Some(bus) = crate::get_event_bus() {
            bus.emit(
                "session:title_updated",
                serde_json::json!({
                    "sessionId": session_id,
                    "title": title,
                }),
            );
        }
        return Ok(Some(title));
    }
    Ok(None)
}

// ── Database path helper ─────────────────────────────────────────

/// Get the database file path: ~/.hope-agent/sessions.db
pub fn db_path() -> Result<PathBuf> {
    Ok(crate::paths::root_dir()?.join("sessions.db"))
}

/// Resolve session metadata from the globally-registered SessionDB.
/// Returns `None` when the global DB is not initialized, the session is
/// missing, or the lookup fails.
pub fn lookup_session_meta(session_id: Option<&str>) -> Option<SessionMeta> {
    let sid = session_id?;
    let db = crate::get_session_db()?;
    db.get_session(sid).ok().flatten()
}

/// Whether the given session is running in incognito mode.
///
/// **Fail-closed three-state** (Epic E / INCOG-1). A late-arriving operation
/// (memory extraction, large-result disk persistence, async-job spool) must
/// never leave a trace for a session that was burned on close, so the three DB
/// outcomes are deliberately *not* collapsed into one `false` like the generic
/// [`lookup_session_meta`] helper does:
///   - **DB not initialized** (early startup / unit tests) → `false`: no
///     incognito session can exist before the store is up, so this is safe.
///   - **Row genuinely absent** (`Ok(None)`) → `true` (**fail-closed**): a live
///     session always has its row, so an absent row means it was deleted or
///     burned (incognito close physically removes it). Any trailing work must
///     be treated as incognito and skip every persistence sidecar.
///   - **Transient lookup error** (lock contention / IO) → `false` + warn: a
///     momentary glitch must NOT silently drop a *normal* session's memory
///     extraction & persistence. The privacy-critical burn path is additionally
///     guarded by the watcher purge ([`super::cleanup_watcher`]) and the
///     frontend best-effort cancel.
pub fn is_session_incognito(session_id: Option<&str>) -> bool {
    let Some(sid) = session_id else {
        return false;
    };
    let Some(db) = crate::get_session_db() else {
        // DB not initialized — no incognito sessions can exist yet.
        return false;
    };
    match db.get_session(sid) {
        Ok(Some(meta)) => meta.incognito,
        // Row gone (deleted / incognito-burned) — fail closed.
        Ok(None) => true,
        Err(e) => {
            crate::app_warn!(
                "session",
                "is_session_incognito",
                "meta lookup for {} failed, treating as non-incognito: {}",
                sid,
                e
            );
            false
        }
    }
}

/// Resolve the effective working directory for a session: session-level value
/// if set, otherwise the parent project's directory (its explicitly selected
/// `working_dir`, or its lazily-created default workspace). This is the single
/// source of truth consumed by both system-prompt rendering and tool execution
/// context, so the model's view and the tool runtime never disagree (write_file
/// allowlists, exec cwd, file mention, etc.).
///
/// Any session attached to a project resolves to `Some(<existing dir>)`; only
/// sessions with neither a session-level working dir nor a project return
/// `None` (unchanged pre-project behavior).
pub fn effective_session_working_dir(session_id: Option<&str>) -> Option<String> {
    let meta = lookup_session_meta(session_id)?;
    effective_working_dir_for_meta(&meta)
}

/// Same resolution as [`effective_session_working_dir`] but for a caller that
/// already holds the [`SessionMeta`], avoiding a redundant DB lookup.
pub fn effective_working_dir_for_meta(meta: &SessionMeta) -> Option<String> {
    if let Some(wd) = meta.working_dir.clone().filter(|s| !s.trim().is_empty()) {
        return Some(wd);
    }
    // 设计线程（kind=Design）无 project_id：其工作目录 = 所属设计项目绑定的代码仓库，
    // **实时派生**（HA 项目 working_dir 变更 / 绑定切换 / 解绑立即反映，绝不 stale）。
    // 无绑定则继续走下面的 project 分支（通常 None → 无工作目录段）。design 库句柄缓存，
    // 非设计会话经 kind 短路零成本（review F3/F5/F6：拆事件拷贝、改实时派生）。
    if meta.kind == crate::session::SessionKind::Design {
        if let Some(dir) = crate::design::service::session_bound_code_dir(&meta.id) {
            return Some(dir);
        }
    }
    let pid = meta.project_id.as_deref()?;
    // An explicit project `working_dir` wins — but a missing project row or a
    // transient DB error must NOT silently drop the session to the agent home
    // (which would scatter the model's relative writes). Fall through to the
    // project's default workspace, which only needs the id.
    if let Some(db) = crate::get_project_db() {
        match db.get(pid) {
            Ok(Some(project)) => {
                if let Some(wd) = project.working_dir.filter(|s| !s.trim().is_empty()) {
                    return Some(wd);
                }
            }
            Ok(None) => {}
            Err(e) => {
                crate::app_warn!(
                    "session",
                    "resolve_working_dir",
                    "project {} lookup failed, falling back to default workspace: {}",
                    pid,
                    e
                );
            }
        }
    }
    // No explicit working dir (or an unreadable row) → lazily materialize the
    // default workspace and use it. Failure degrades to `None` (no working-dir
    // section injected) rather than panicking.
    let ws = crate::paths::project_workspace_dir(pid).ok()?;
    match crate::util::ensure_dir_canonical(&ws) {
        Ok(path) => Some(path),
        Err(e) => {
            crate::app_warn!(
                "session",
                "ensure_workspace",
                "failed to create default workspace for project {}: {}",
                pid,
                e
            );
            None
        }
    }
}

// ── Startup recovery ────────────────────────────────────────────

/// Sweep incognito sessions left behind from a previous run (crash, SIGKILL,
/// power loss). Same shape as `subagent::cleanup_orphan_runs` and
/// `team::cleanup::cleanup_orphan_teams` — `app_init` calls all three back to
/// back. Failures are warned, never propagated.
pub fn cleanup_orphan_incognito(session_db: &super::SessionDB) {
    if let Err(e) = session_db.purge_orphan_incognito_sessions() {
        crate::app_warn!(
            "session",
            "purge_orphan_incognito",
            "startup sweep failed: {}",
            e
        );
    }
}
