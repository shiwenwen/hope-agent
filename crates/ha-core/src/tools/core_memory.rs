use anyhow::Result;
use serde_json::{json, Value};

use super::ToolExecContext;
use crate::memory::core_repository::{self, CoreMemoryScope, CoreMemoryTopicWriteInput};

pub(crate) async fn tool_core_memory(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    if ctx.incognito {
        anyhow::bail!("core_memory is unavailable in an incognito session");
    }
    let runtime = crate::config::cached_config().memory.clone();
    let memory_enabled = runtime.effective_enabled(crate::memory::load_extract_config().enabled);
    if !memory_enabled || (runtime.rollout.enabled && !runtime.core.enabled) {
        anyhow::bail!("core_memory is unavailable because Memory is turned off");
    }
    let agent_id = ctx
        .agent_id
        .as_deref()
        .unwrap_or(crate::agent_loader::DEFAULT_AGENT_ID);
    let agent_definition = crate::agent_loader::load_agent(agent_id).ok();
    if agent_definition
        .as_ref()
        .is_some_and(|definition| !definition.config.memory.enabled)
    {
        anyhow::bail!("core_memory is disabled for the current Agent");
    }
    let shared_global = agent_definition
        .as_ref()
        .is_some_and(|definition| definition.config.memory.shared);
    let scope = resolve_scope(
        args.get("scope").and_then(Value::as_str).unwrap_or("agent"),
        agent_id,
        ctx.project_id.as_deref(),
        shared_global,
    )?;
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?
        .to_string();
    match action.as_str() {
        "get_index" | "list" | "read" | "search" | "reload" => {
            super::memory::ensure_session_memory_read(ctx, "core_memory")?;
        }
        "append_index" | "replace_index" | "write" | "delete" | "rebuild_index" | "promote" => {
            super::memory::ensure_session_memory_contribution(ctx, "core_memory")?;
        }
        _ => {}
    }
    let args = args.clone();
    let operation_action = action.clone();
    let session_id = ctx.session_id.clone();
    let source_agent_id = agent_id.to_string();
    let source_project_id = ctx.project_id.clone();
    let topic_read_token_cap = runtime.core.topic_read_max_tokens as usize;

    let result = crate::blocking::run_blocking(move || -> Result<Value> {
        match operation_action.as_str() {
            "get_index" => Ok(serde_json::to_value(core_repository::load_index(&scope)?)?),
            "append_index" => {
                let content = required_string(&args, "content")?;
                let current = core_repository::load_index(&scope)?;
                let next = match current.content.as_deref().map(str::trim_end) {
                    Some(existing) if !existing.is_empty() => format!("{existing}\n{content}"),
                    _ => content.to_string(),
                };
                Ok(serde_json::to_value(core_repository::save_index(
                    &scope,
                    &next,
                    current.file_hash.as_deref(),
                )?)?)
            }
            "replace_index" => {
                let content = required_string(&args, "content")?;
                let expected = args.get("expectedFileHash").and_then(Value::as_str);
                Ok(serde_json::to_value(core_repository::save_index(
                    &scope, content, expected,
                )?)?)
            }
            "list" => Ok(serde_json::to_value(core_repository::list_topics_page(
                &scope,
                args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
                args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize,
            )?)?),
            "read" => {
                let file_name = required_string(&args, "fileName")?;
                let file = core_repository::read_topic(&scope, file_name)?;
                let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
                let requested = args
                    .get("maxChars")
                    .and_then(Value::as_u64)
                    .unwrap_or(20_000) as usize;
                let max_chars = requested.clamp(100, 20_000);
                let total_chars = file.content.chars().count();
                let content =
                    take_topic_content(&file.content, offset, max_chars, topic_read_token_cap);
                let returned_chars = content.chars().count();
                Ok(json!({
                    "entry": file.entry,
                    "fileHash": file.file_hash,
                    "content": content,
                    "offset": offset.min(total_chars),
                    "returnedChars": returned_chars,
                    "totalChars": total_chars,
                    "truncated": offset.saturating_add(returned_chars) < total_chars,
                }))
            }
            "search" => Ok(serde_json::to_value(core_repository::search_topics(
                &scope,
                required_string(&args, "query")?,
                args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize,
            )?)?),
            "write" => Ok(serde_json::to_value(core_repository::write_topic(
                &scope,
                CoreMemoryTopicWriteInput {
                    file_name: args
                        .get("fileName")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    expected_file_hash: args
                        .get("expectedFileHash")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    name: required_string(&args, "name")?.to_string(),
                    description: required_string(&args, "description")?.to_string(),
                    memory_type: args
                        .get("memoryType")
                        .and_then(Value::as_str)
                        .unwrap_or("project")
                        .to_string(),
                    content: required_string(&args, "content")?.to_string(),
                },
            )?)?),
            "delete" => Ok(json!({
                "deleted": core_repository::delete_topic(
                    &scope,
                    required_string(&args, "fileName")?,
                    args.get("expectedFileHash").and_then(Value::as_str),
                )?
            })),
            "rebuild_index" => Ok(json!({
                "index": core_repository::rebuild_topic_index(&scope)?,
                "topics": core_repository::list_topics(&scope)?.len(),
            })),
            "promote" => {
                let source_kind = match required_string(&args, "sourceKind")? {
                    "memory" => core_repository::CoreMemoryPromotionSourceKind::Memory,
                    "claim" => core_repository::CoreMemoryPromotionSourceKind::Claim,
                    other => anyhow::bail!("invalid promotion source kind: {other}"),
                };
                ensure_promotion_source_visible(
                    source_kind,
                    required_string(&args, "sourceId")?,
                    &source_agent_id,
                    source_project_id.as_deref(),
                    shared_global,
                )?;
                Ok(serde_json::to_value(core_repository::promote(
                    core_repository::CoreMemoryPromotionInput {
                        source_kind,
                        source_id: required_string(&args, "sourceId")?.to_string(),
                        scope_type: scope.scope_type().to_string(),
                        scope_id: scope.scope_id().map(str::to_string),
                        topic_name: args
                            .get("topicName")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    },
                )?)?)
            }
            "reload" => {
                let session_id = session_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("core_memory reload requires a session"))?;
                core_repository::invalidate_session_snapshot(session_id);
                if let Some(bus) = crate::get_event_bus() {
                    bus.emit(
                        "memory:core_snapshot_reloaded",
                        json!({ "sessionId": session_id }),
                    );
                }
                Ok(json!({ "reloaded": true }))
            }
            other => anyhow::bail!("invalid core_memory action: {other}"),
        }
    })
    .await?;

    serde_json::to_string_pretty(&result).map_err(Into::into)
}

fn ensure_promotion_source_visible(
    source_kind: core_repository::CoreMemoryPromotionSourceKind,
    source_id: &str,
    agent_id: &str,
    project_id: Option<&str>,
    shared_global: bool,
) -> Result<()> {
    let allowed = |scope_type: &str, scope_id: Option<&str>| {
        promotion_scope_visible(scope_type, scope_id, agent_id, project_id, shared_global)
    };
    let visible = match source_kind {
        core_repository::CoreMemoryPromotionSourceKind::Memory => {
            let memory_id = source_id
                .parse::<i64>()
                .map_err(|_| anyhow::anyhow!("memory sourceId must be an integer"))?;
            let backend = crate::get_memory_backend()
                .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;
            let memory = backend
                .get(memory_id)?
                .ok_or_else(|| anyhow::anyhow!("memory source not found"))?;
            match &memory.scope {
                crate::memory::MemoryScope::Global => allowed("global", None),
                crate::memory::MemoryScope::Agent { id } => allowed("agent", Some(id)),
                crate::memory::MemoryScope::Project { id } => allowed("project", Some(id)),
            }
        }
        core_repository::CoreMemoryPromotionSourceKind::Claim => {
            let detail = crate::memory::claims::get_claim(source_id)?
                .ok_or_else(|| anyhow::anyhow!("claim source not found"))?;
            allowed(&detail.claim.scope_type, detail.claim.scope_id.as_deref())
        }
    };
    if !visible {
        anyhow::bail!("Core Memory promotion source is outside the current session scope");
    }
    Ok(())
}

fn promotion_scope_visible(
    scope_type: &str,
    scope_id: Option<&str>,
    agent_id: &str,
    project_id: Option<&str>,
    shared_global: bool,
) -> bool {
    match scope_type {
        "global" => shared_global,
        "agent" => scope_id == Some(agent_id),
        "project" => project_id.is_some() && scope_id == project_id,
        _ => false,
    }
}

fn resolve_scope(
    requested: &str,
    agent_id: &str,
    project_id: Option<&str>,
    shared_global: bool,
) -> Result<CoreMemoryScope> {
    match requested {
        "global" if shared_global => Ok(CoreMemoryScope::Global),
        "global" => anyhow::bail!("global Core Memory is disabled for the current Agent"),
        "agent" => Ok(CoreMemoryScope::Agent {
            id: agent_id.to_string(),
        }),
        "project" => project_id
            .map(|id| CoreMemoryScope::Project { id: id.to_string() })
            .ok_or_else(|| anyhow::anyhow!("project Core Memory requires a project-bound session")),
        other => anyhow::bail!("invalid Core Memory scope: {other}"),
    }
}

fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required parameter: {key}"))
}

fn take_topic_content(content: &str, offset: usize, max_chars: usize, max_tokens: usize) -> String {
    let mut output = String::new();
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for character in content.chars().skip(offset).take(max_chars) {
        let (next_ascii, next_non_ascii) = if character.is_ascii() {
            (ascii + 1, non_ascii)
        } else {
            (ascii, non_ascii + 1)
        };
        let base = next_ascii
            .div_ceil(3)
            .saturating_add(next_non_ascii.saturating_mul(2));
        let estimate = base.saturating_mul(11).div_ceil(10).max(1);
        if estimate > max_tokens {
            break;
        }
        output.push(character);
        ascii = next_ascii;
        non_ascii = next_non_ascii;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_scope_never_accepts_an_arbitrary_id() {
        assert!(resolve_scope("project", "ha-main", None, true).is_err());
        assert_eq!(
            resolve_scope(
                "project",
                "ha-main",
                Some("00000000-0000-0000-0000-000000000001"),
                true,
            )
            .unwrap()
            .key(),
            "project:00000000-0000-0000-0000-000000000001"
        );
    }

    #[test]
    fn global_scope_follows_the_agent_shared_memory_switch() {
        assert!(resolve_scope("global", "ha-main", None, false).is_err());
        assert!(matches!(
            resolve_scope("global", "ha-main", None, true).unwrap(),
            CoreMemoryScope::Global
        ));
    }

    #[test]
    fn promotion_scope_gate_never_leaks_other_agent_or_project_sources() {
        let visible = |scope_type: &str, scope_id: Option<&str>| {
            promotion_scope_visible(
                scope_type,
                scope_id,
                "ha-main",
                Some("00000000-0000-0000-0000-000000000001"),
                false,
            )
        };
        assert!(visible("agent", Some("ha-main")));
        assert!(!visible("agent", Some("other")));
        assert!(visible(
            "project",
            Some("00000000-0000-0000-0000-000000000001")
        ));
        assert!(!visible(
            "project",
            Some("00000000-0000-0000-0000-000000000002")
        ));
        assert!(!visible("global", None));
    }

    #[test]
    fn topic_read_budget_is_conservative_for_ascii_and_cjk() {
        let ascii = take_topic_content(&"a".repeat(10_000), 0, 10_000, 100);
        let cjk = take_topic_content(&"记".repeat(10_000), 0, 10_000, 100);

        assert!(crate::system_prompt::conservative_core_token_estimate(&ascii) <= 100);
        assert!(crate::system_prompt::conservative_core_token_estimate(&cjk) <= 100);
        assert!(cjk.chars().count() < ascii.chars().count());
        assert_eq!(take_topic_content("012345", 2, 2, 100), "23");
    }
}
