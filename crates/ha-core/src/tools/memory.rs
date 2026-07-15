use anyhow::Result;
use serde_json::Value;

use crate::memory::{self, AddResult, MemoryScope, MemorySearchQuery, MemoryType, NewMemory};

const SAVE_MEMORY_SOURCE: &str = "user";

#[derive(Debug, Clone)]
struct AgentMemoryScopeAccess {
    agent_id: String,
    project_id: Option<String>,
    shared_global: bool,
}

impl AgentMemoryScopeAccess {
    fn from_context(ctx: &super::ToolExecContext) -> Result<Self> {
        let agent_id = ctx
            .agent_id
            .clone()
            .unwrap_or_else(|| crate::agent_loader::DEFAULT_AGENT_ID.to_string());
        let definition = crate::agent_loader::load_agent(&agent_id).map_err(|error| {
            anyhow::anyhow!("failed to load current Agent memory policy: {error}")
        })?;
        if !definition.config.memory.enabled {
            anyhow::bail!("Memory is disabled for the current Agent");
        }
        Ok(Self {
            agent_id,
            project_id: ctx.project_id.clone(),
            shared_global: definition.config.memory.shared,
        })
    }

    fn readable_scopes(&self) -> Vec<MemoryScope> {
        let mut scopes = Vec::with_capacity(3);
        if let Some(project_id) = &self.project_id {
            scopes.push(MemoryScope::Project {
                id: project_id.clone(),
            });
        }
        scopes.push(MemoryScope::Agent {
            id: self.agent_id.clone(),
        });
        if self.shared_global {
            scopes.push(MemoryScope::Global);
        }
        scopes
    }

    fn allows(&self, scope: &MemoryScope) -> bool {
        match scope {
            MemoryScope::Global => self.shared_global,
            MemoryScope::Agent { id } => id == &self.agent_id,
            MemoryScope::Project { id } => self.project_id.as_ref() == Some(id),
        }
    }

    fn ensure_allows(&self, scope: &MemoryScope) -> Result<()> {
        if !self.allows(scope) {
            anyhow::bail!("memory is outside the current Agent/Project scope");
        }
        Ok(())
    }
}

pub(crate) fn ensure_session_memory_read(ctx: &super::ToolExecContext, tool: &str) -> Result<()> {
    let access = memory::effective_session_memory_access(
        ctx.session_id.as_deref(),
        ctx.session_db.as_ref().map(|handle| handle.0.as_ref()),
    );
    if !access.use_memories {
        anyhow::bail!(
            "{tool} is unavailable because this session is configured not to use memories"
        );
    }
    Ok(())
}

pub(crate) fn ensure_session_memory_contribution(
    ctx: &super::ToolExecContext,
    tool: &str,
) -> Result<()> {
    let access = memory::effective_session_memory_access(
        ctx.session_id.as_deref(),
        ctx.session_db.as_ref().map(|handle| handle.0.as_ref()),
    );
    if !access.contribute_to_memories {
        anyhow::bail!(
            "{tool} is unavailable because this session is configured not to contribute to memories"
        );
    }
    Ok(())
}

/// Tool: save_memory — persist information for future conversations.
///
/// When the active session belongs to a project and the model did not pass
/// an explicit `scope` argument, the new memory defaults to the Project
/// scope so it stays inside that project. This mirrors the behavior of
/// `memory_extract::resolve_extract_scope` so manual and auto-extracted
/// memories land in the same place for project sessions.
pub(crate) async fn tool_save_memory(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    // Incognito red-line: a burn-on-close session must never persist into the
    // long-term memory store. The extraction paths (memory_extract) and the KB
    // note tools already fail-closed on incognito; this is the matching guard
    // for the manual write tool, which otherwise wrote straight to memory.db.
    if ctx.incognito {
        return Err(anyhow::anyhow!(
            "save_memory is unavailable in an incognito session (close = burn)"
        ));
    }
    ensure_session_memory_contribution(ctx, "save_memory")?;
    let scope_access = AgentMemoryScopeAccess::from_context(ctx)?;
    if args
        .get("agent_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id != scope_access.agent_id)
    {
        anyhow::bail!("save_memory cannot write another Agent's memories");
    }
    if args
        .get("project_id")
        .and_then(Value::as_str)
        .is_some_and(|id| scope_access.project_id.as_deref() != Some(id))
    {
        anyhow::bail!("save_memory cannot write an unbound Project's memories");
    }
    let runtime = crate::config::cached_config().memory.clone();
    let memory_enabled = runtime.effective_enabled(memory::load_extract_config().enabled);
    if !memory_enabled {
        return Err(anyhow::anyhow!(
            "save_memory is unavailable because long-term memory is turned off"
        ));
    }

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

    let memory_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("user");

    // Detect the current session's project via ctx so we can default
    // project-session memories to the right scope without the model having
    // to pass `scope="project"` and `project_id` every time.
    let session_project_id = scope_access.project_id.clone();

    // Resolve the scope string. When the model omits `scope`:
    //   * session is in a project → Project scope (so knowledge stays local)
    //   * otherwise, V2            → current Agent scope
    //   * otherwise, legacy        → Global scope (rollback compatibility)
    let explicit_scope = args.get("scope").and_then(|v| v.as_str());
    let default_scope = if session_project_id.is_some() {
        "project"
    } else if runtime.rollout.enabled {
        "agent"
    } else {
        "global"
    };
    let scope_str = explicit_scope.unwrap_or(default_scope);

    let agent_id = scope_access.agent_id.as_str();

    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let pinned = args
        .get("pinned")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Resolve scope string. `project` requires a project context, either
    // passed explicitly via `project_id` or reachable via the current
    // session (looked up from the global session DB / ctx).
    let scope = match scope_str {
        "agent" => MemoryScope::Agent {
            id: agent_id.to_string(),
        },
        "project" => match session_project_id.clone() {
            Some(id) => MemoryScope::Project { id },
            None => {
                return Err(anyhow::anyhow!(
                    "scope=project requires a project-bound current session"
                ));
            }
        },
        "global" => MemoryScope::Global,
        other => anyhow::bail!("Invalid scope: '{other}'. Use 'agent', 'project', or 'global'."),
    };
    scope_access.ensure_allows(&scope)?;

    let entry = NewMemory {
        memory_type: MemoryType::from_str(memory_type),
        scope,
        content: content.to_string(),
        tags,
        source: SAVE_MEMORY_SOURCE.to_string(),
        source_session_id: ctx.session_id.clone(),
        pinned,
        attachment_path: None,
        attachment_mime: None,
    };

    // Run blocking backend operations (embedding API + SQLite) on a blocking thread
    // to avoid blocking the tokio runtime.
    let memory_type = memory_type.to_string();
    let scope_str = scope_str.to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let backend = crate::get_memory_backend()
            .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

        let dedup = memory::load_dedup_config();
        match backend.add_with_dedup(entry, dedup.threshold_high, dedup.threshold_merge)? {
            AddResult::Created { id } => Ok(format!(
                "Memory saved (id: {}, type: {}, scope: {})",
                id, memory_type, scope_str
            )),
            AddResult::Duplicate { existing_id, score } => Ok(format!(
                "Similar memory already exists (id: {}, similarity: {:.1}%). Not saved.",
                existing_id,
                score * 100.0
            )),
            AddResult::Updated { id } => Ok(format!(
                "Merged with existing memory (id: {}, type: {}, scope: {})",
                id, memory_type, scope_str
            )),
        }
    })
    .await??;

    Ok(result)
}

/// Tool: recall_memory — search persistent memories by keyword or semantic query.
/// Optionally also searches past conversation history (include_history=true).
///
/// Phase B'3: when `AppConfig.recall_summary.enabled` AND the total hits
/// (memories + optional history) meet `min_hits`, the raw snippet output is
/// collapsed into a single concise paragraph via a bounded `side_query`.
/// Failures degrade silently back to the raw output.
pub(crate) async fn tool_recall_memory(
    args: &Value,
    ctx: &super::ToolExecContext,
) -> Result<String> {
    ensure_session_memory_read(ctx, "recall_memory")?;
    let scope_access = AgentMemoryScopeAccess::from_context(ctx)?;
    let query_text = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?
        .to_string();

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    let type_filter = args
        .get("type")
        .and_then(|v| v.as_str())
        .map(|t| vec![MemoryType::from_str(t)]);

    if args
        .get("agent_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id != scope_access.agent_id)
    {
        anyhow::bail!("recall_memory cannot search another Agent's memories");
    }

    let include_history = args
        .get("include_history")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Run blocking backend operations (embedding API + SQLite) on a blocking thread
    // to avoid blocking the tokio runtime. We also return the hit count so
    // the async caller can decide whether to summarize.
    let query_text_for_blocking = query_text.clone();
    let query_text_for_search = query_text.clone();
    let agent_id = scope_access.agent_id.clone();
    let readable_scopes = scope_access.readable_scopes();
    let bound_session_db = ctx.session_db.as_ref().map(|handle| handle.0.clone());

    let (raw_output, total_hits) =
        tokio::task::spawn_blocking(move || -> Result<(String, usize)> {
            let backend = crate::get_memory_backend()
                .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

            let mut results = Vec::new();
            let mut seen_ids = std::collections::HashSet::new();
            for scope in readable_scopes {
                let query = MemorySearchQuery {
                    query: query_text_for_blocking.clone(),
                    types: type_filter.clone(),
                    sources: None,
                    scope: Some(scope),
                    agent_id: None,
                    limit: Some(limit),
                };
                for memory in backend.search(&query)? {
                    if seen_ids.insert(memory.id) {
                        results.push(memory);
                        if results.len() >= limit {
                            break;
                        }
                    }
                }
                if results.len() >= limit {
                    break;
                }
            }

            let mut output = String::new();
            let mem_count = results.len();
            let mut hist_count = 0usize;

            if !results.is_empty() {
                output.push_str(&format!("Found {} memories:\n\n", results.len()));
                for (i, mem) in results.iter().enumerate() {
                    let scope_label = match &mem.scope {
                        MemoryScope::Global => "global".to_string(),
                        MemoryScope::Agent { id } => format!("agent:{}", id),
                        MemoryScope::Project { id } => format!("project:{}", id),
                    };
                    let pin_marker = if mem.pinned { "★ " } else { "" };
                    let tags_str = if mem.tags.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", mem.tags.join(", "))
                    };
                    output.push_str(&format!(
                        "{}. {}(id: {}) [{}|{}]{}\n{}\n\n",
                        i + 1,
                        pin_marker,
                        mem.id,
                        mem.memory_type.as_str(),
                        scope_label,
                        tags_str,
                        mem.content,
                    ));
                }
            }

            // Search conversation history if requested
            if include_history {
                let global_session_db = crate::get_session_db();
                if let Some(session_db) = bound_session_db
                    .as_deref()
                    .or_else(|| global_session_db.map(|db| db.as_ref()))
                {
                    let history_results = session_db
                        .search_message_content(
                            &query_text_for_search,
                            Some(&agent_id),
                            None,
                            None,
                            5,
                        )
                        .unwrap_or_default();

                    if !history_results.is_empty() {
                        hist_count = history_results.len();
                        output.push_str(&format!(
                            "\n--- Conversation History ({} matches) ---\n\n",
                            history_results.len()
                        ));
                        for (i, hit) in history_results.iter().enumerate() {
                            let session_label = hit.session_title.as_deref().unwrap_or("Untitled");
                            output.push_str(&format!(
                                "{}. [{}] {} (session: {}, {})\n{}\n\n",
                                i + 1,
                                hit.message_role,
                                hit.timestamp,
                                session_label,
                                hit.session_id,
                                hit.content_snippet,
                            ));
                        }
                    }
                }
            }

            if output.is_empty() {
                return Ok((
                    "No memories or history found matching the query.".to_string(),
                    0,
                ));
            }

            Ok((output, mem_count + hist_count))
        })
        .await??;

    // Phase B'4 learning event: count every non-empty recall as a hit.
    if total_hits > 0 {
        crate::dashboard::emit_learning_event(
            crate::dashboard::EVT_RECALL_HIT,
            None,
            None,
            Some(&serde_json::json!({
                "hits": total_hits,
                "query_chars": query_text.chars().count(),
            })),
        );
    }

    // Phase B'3: optional LLM-summarization layer over the raw snippet
    // output. Opt-in via `AppConfig.recall_summary.enabled`. `total_hits` is
    // already memory-only when `include_history=false` (history snippets are
    // guarded by that flag at fetch time), so we don't need a second gate here.
    let cfg = crate::config::cached_config().recall_summary.clone();
    if let Some(summary) =
        crate::memory::maybe_summarize_recall(&query_text, total_hits, &raw_output, &cfg).await
    {
        crate::dashboard::emit_learning_event(
            crate::dashboard::EVT_RECALL_SUMMARY_USED,
            None,
            None,
            Some(&serde_json::json!({ "hits": total_hits })),
        );
        return Ok(format!(
            "## Summary of {} hits\n\n{}\n\n---\nRaw hits suppressed (recall_summary enabled). Original count: {}",
            total_hits, summary, total_hits
        ));
    }

    Ok(raw_output)
}

/// Tool: update_memory — update an existing memory's content and/or tags.
pub(crate) async fn tool_update_memory(
    args: &Value,
    ctx: &super::ToolExecContext,
) -> Result<String> {
    ensure_session_memory_contribution(ctx, "update_memory")?;
    let scope_access = AgentMemoryScopeAccess::from_context(ctx)?;
    let id = args
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter (integer)"))?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?
        .to_string();

    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Run blocking backend operations on a blocking thread.
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let backend = crate::get_memory_backend()
            .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

        let Some(existing) = backend.get(id)? else {
            return Ok(format!("Memory with id {} not found.", id));
        };
        scope_access.ensure_allows(&existing.scope)?;

        backend.update(id, &content, &tags)?;

        Ok(format!("Memory updated (id: {}).", id))
    })
    .await??;

    Ok(result)
}

/// Tool: delete_memory — remove a memory by its ID.
pub(crate) async fn tool_delete_memory(
    args: &Value,
    ctx: &super::ToolExecContext,
) -> Result<String> {
    ensure_session_memory_contribution(ctx, "delete_memory")?;
    let scope_access = AgentMemoryScopeAccess::from_context(ctx)?;
    let id = args
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter (integer)"))?;

    // Run blocking backend operations on a blocking thread.
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let backend = crate::get_memory_backend()
            .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

        let Some(existing) = backend.get(id)? else {
            return Ok(format!("Memory with id {} not found.", id));
        };
        scope_access.ensure_allows(&existing.scope)?;

        backend.delete(id)?;

        Ok(format!("Memory deleted (id: {}).", id))
    })
    .await??;

    Ok(result)
}

/// Tool: memory_get — retrieve a specific memory entry by ID with full content and metadata.
pub(crate) async fn tool_memory_get(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    ensure_session_memory_read(ctx, "memory_get")?;
    let scope_access = AgentMemoryScopeAccess::from_context(ctx)?;
    let id = args
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter (integer)"))?;

    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let backend = crate::get_memory_backend()
            .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

        match backend.get(id)? {
            Some(mem) => {
                scope_access.ensure_allows(&mem.scope)?;
                let scope_label = match &mem.scope {
                    MemoryScope::Global => "global".to_string(),
                    MemoryScope::Agent { id } => format!("agent:{}", id),
                    MemoryScope::Project { id } => format!("project:{}", id),
                };
                let tags_str = if mem.tags.is_empty() {
                    String::new()
                } else {
                    format!(" tags: [{}]", mem.tags.join(", "))
                };
                Ok(format!(
                    "Memory #{} [{}|{}]{}\nSource: {} | Created: {} | Updated: {}\n\n{}",
                    mem.id,
                    mem.memory_type.as_str(),
                    scope_label,
                    tags_str,
                    mem.source,
                    mem.created_at,
                    mem.updated_at,
                    mem.content,
                ))
            }
            None => Ok(format!("Memory with id {} not found.", id)),
        }
    })
    .await??;

    Ok(result)
}

/// Tool: update_core_memory — compatibility alias for the Core `MEMORY.md` index
/// in the system prompt. Used for persistent rules, preferences, and standing instructions.
pub(crate) async fn tool_update_core_memory(
    args: &Value,
    ctx: &super::ToolExecContext,
) -> Result<String> {
    // Incognito red-line: never write Core Memory from a
    // burn-on-close session. Mirrors save_memory + the extraction guards.
    if ctx.incognito {
        return Err(anyhow::anyhow!(
            "update_core_memory is unavailable in an incognito session (close = burn)"
        ));
    }
    ensure_session_memory_contribution(ctx, "update_core_memory")?;
    let runtime = crate::config::cached_config().memory.clone();
    let memory_enabled = runtime.effective_enabled(memory::load_extract_config().enabled);
    if !memory_enabled || (runtime.rollout.enabled && !runtime.core.enabled) {
        return Err(anyhow::anyhow!(
            "update_core_memory is unavailable because long-term memory is turned off"
        ));
    }

    let agent_id = ctx
        .agent_id
        .as_deref()
        .unwrap_or(crate::agent_loader::DEFAULT_AGENT_ID);
    let agent_memory = crate::agent_loader::load_agent(agent_id)
        .map(|definition| definition.config.memory)
        .map_err(|error| anyhow::anyhow!("failed to load current Agent memory policy: {error}"))?;
    if !agent_memory.enabled {
        anyhow::bail!("update_core_memory is disabled for the current Agent");
    }

    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("append");

    let scope = args
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("agent");

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

    let repository_scope = match scope {
        "global" if agent_memory.shared => crate::memory::core_repository::CoreMemoryScope::Global,
        "global" => anyhow::bail!("global Core Memory is disabled for the current Agent"),
        "agent" => crate::memory::core_repository::CoreMemoryScope::Agent {
            id: agent_id.to_string(),
        },
        other => anyhow::bail!("Invalid scope: '{other}'. Use 'agent' or 'global'."),
    };

    let action_owned = action.to_string();
    let scope_owned = scope.to_string();
    let agent_id_owned = agent_id.to_string();
    let content_owned = content.to_string();

    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let current = crate::memory::core_repository::load_index(&repository_scope)?;
        let existing = current.content.unwrap_or_default();
        let new_content = match action_owned.as_str() {
            "append" if existing.trim().is_empty() => content_owned.clone(),
            "append" => format!("{}\n{}", existing.trim_end(), content_owned),
            "replace" => content_owned.clone(),
            other => anyhow::bail!("Invalid action: '{}'. Use 'append' or 'replace'.", other),
        };
        crate::memory::core_repository::save_index(
            &repository_scope,
            &new_content,
            current.file_hash.as_deref(),
        )?;

        // Emit event to notify frontend
        if let Some(bus) = crate::globals::get_event_bus() {
            bus.emit(
                "core_memory_updated",
                serde_json::json!({
                    "agentId": agent_id_owned,
                    "scope": scope_owned,
                }),
            );
        }

        Ok(format!(
            "Core memory updated (action: {}, scope: {})",
            action_owned, scope_owned
        ))
    })
    .await??;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Incognito red-line: the manual memory-write tools must refuse to persist
    /// anything from a burn-on-close session. The guard runs before any backend
    /// / filesystem access, so this needs no initialised memory backend.
    #[tokio::test]
    async fn save_memory_refuses_incognito_session() {
        let ctx = super::super::ToolExecContext {
            incognito: true,
            ..Default::default()
        };
        let err = tool_save_memory(&json!({ "content": "secret" }), &ctx)
            .await
            .expect_err("save_memory must be refused in an incognito session");
        assert!(
            err.to_string().contains("incognito"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn update_core_memory_refuses_incognito_session() {
        let ctx = super::super::ToolExecContext {
            incognito: true,
            ..Default::default()
        };
        let err = tool_update_core_memory(&json!({ "content": "rule", "action": "append" }), &ctx)
            .await
            .expect_err("update_core_memory must be refused in an incognito session");
        assert!(
            err.to_string().contains("incognito"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn save_memory_uses_manual_source_label() {
        assert_eq!(SAVE_MEMORY_SOURCE, "user");
    }

    #[test]
    fn dynamic_memory_scope_gate_is_project_agent_global_and_fail_closed() {
        let access = AgentMemoryScopeAccess {
            agent_id: "ha-main".into(),
            project_id: Some("00000000-0000-0000-0000-000000000001".into()),
            shared_global: false,
        };
        assert!(access.allows(&MemoryScope::Agent {
            id: "ha-main".into()
        }));
        assert!(!access.allows(&MemoryScope::Agent { id: "other".into() }));
        assert!(access.allows(&MemoryScope::Project {
            id: "00000000-0000-0000-0000-000000000001".into()
        }));
        assert!(!access.allows(&MemoryScope::Project {
            id: "00000000-0000-0000-0000-000000000002".into()
        }));
        assert!(!access.allows(&MemoryScope::Global));
        assert_eq!(
            access.readable_scopes(),
            vec![
                MemoryScope::Project {
                    id: "00000000-0000-0000-0000-000000000001".into()
                },
                MemoryScope::Agent {
                    id: "ha-main".into()
                },
            ]
        );
    }
}
