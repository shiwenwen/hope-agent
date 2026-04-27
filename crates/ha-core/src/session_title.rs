use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::agent::AssistantAgent;
use crate::provider::{ActiveModel, ProviderConfig};
use crate::session::SessionDB;

pub const TITLE_SOURCE_FIRST_MESSAGE: &str = "first_message";
pub const TITLE_SOURCE_LLM: &str = "llm";
pub const TITLE_SOURCE_MANUAL: &str = "manual";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTitleConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

pub fn maybe_schedule_after_success(
    db: Arc<SessionDB>,
    session_id: String,
    agent_id: String,
    chat_model: ActiveModel,
    providers: Vec<ProviderConfig>,
) {
    let cfg = crate::config::cached_config().session_title.clone();
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

    if meta.incognito || meta.title_source != TITLE_SOURCE_FIRST_MESSAGE {
        return;
    }

    let configured_model = cfg
        .provider_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .zip(cfg.model_id.as_deref().filter(|s| !s.trim().is_empty()));
    let (provider_id, model_id) = configured_model
        .map(|(provider_id, model_id)| (provider_id.to_string(), model_id.to_string()))
        .unwrap_or_else(|| (chat_model.provider_id.clone(), chat_model.model_id.clone()));

    let provider = match providers.iter().find(|p| p.id == provider_id).cloned() {
        Some(provider) => provider,
        None => {
            app_warn!(
                "session",
                "title_generate",
                "Skipping title generation for session {}: provider '{}' not found",
                session_id,
                provider_id
            );
            return;
        }
    };

    std::thread::spawn(move || {
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
            db, session_id, agent_id, provider, model_id,
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

async fn generate_and_update_title(
    db: Arc<SessionDB>,
    session_id: String,
    agent_id: String,
    provider: ProviderConfig,
    model_id: String,
) -> Result<()> {
    let messages = collect_title_messages(&db, &session_id)?;
    let user_count = messages
        .iter()
        .filter(|line| line.starts_with("User:"))
        .count();
    let has_assistant = messages.iter().any(|line| line.starts_with("Assistant:"));
    if user_count != 1 || !has_assistant {
        return Ok(());
    }

    let prompt = build_title_prompt(&messages);
    let mut agent = AssistantAgent::try_new_from_provider(&provider, &model_id)
        .await?
        .with_failover_context(&provider);
    agent.set_agent_id(&agent_id);
    agent.set_session_id(&session_id);

    let response = agent
        .side_query(&prompt, 64)
        .await
        .map_err(|e| {
            anyhow!(
                "session title side_query failed (provider_id={}, model={}, session={}): {}",
                provider.id,
                model_id,
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
            "Generated LLM title for session {} using {}::{}",
            session_id,
            provider.id,
            model_id
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
        "SELECT role, content
             FROM messages
             WHERE session_id = ?1 AND role IN ('user', 'assistant')
             ORDER BY id ASC
             LIMIT 16",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut lines = Vec::new();
    let mut saw_user = false;
    for row in rows {
        let (role, content) = row?;
        let label = match role.as_str() {
            "user" if !saw_user => {
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
    fn session_title_config_defaults_disabled() {
        let cfg = SessionTitleConfig::default();
        assert!(!cfg.enabled);
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
    fn collect_title_messages_uses_first_turn_when_followup_exists() {
        let db_path = std::env::temp_dir().join(format!(
            "hope-agent-session-title-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = SessionDB::open(&db_path).expect("open session db");
        let session = db.create_session("default").expect("create session");
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
