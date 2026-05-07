use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult, SessionPickerItem};
use std::sync::Arc;

/// Maximum sessions surfaced in the `/sessions` picker. IM platforms cap
/// inline-button payloads, so we keep the list small enough to render as
/// buttons without truncation.
const SESSION_PICKER_LIMIT: usize = 30;

/// FTS5 distinct-session cap. SQL-level dedupe means this is in distinct
/// sessions, not raw messages — set to picker limit since we'll truncate
/// to it anyway after the metadata-match union.
const SESSION_FTS_LIMIT: usize = SESSION_PICKER_LIMIT;

/// Snippet length cap (some IM platforms wrap long lines awkwardly).
const SESSION_PICKER_SNIPPET_BYTES: usize = 160;

/// /new — Create a new session, returning a markdown receipt with the agent
/// name, project (if any), and effective working directory.
pub fn handle_new(session_db: &Arc<SessionDB>, agent_id: &str) -> Result<CommandResult, String> {
    let meta = session_db
        .create_session(agent_id)
        .map_err(|e| e.to_string())?;

    let working_dir = crate::session::effective_session_working_dir(Some(&meta.id));

    let mut lines = vec![format!("✅ New session — agent **{}**", agent_id)];
    if let Some(pid) = meta.project_id.as_deref() {
        if let Some(project) =
            crate::globals::get_project_db().and_then(|db| db.get(pid).ok().flatten())
        {
            lines.push(format!("- Project: **{}**", project.name));
        }
    }
    if let Some(wd) = working_dir.as_deref() {
        lines.push(format!("- Working dir: `{}`", wd));
    }

    Ok(CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::NewSession {
            session_id: meta.id,
        }),
    })
}

/// /clear — Delete current session messages.
pub fn handle_clear(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session to clear")?;
    session_db.delete_session(sid).map_err(|e| e.to_string())?;
    Ok(CommandResult {
        content: "Session cleared.".into(),
        action: Some(CommandAction::SessionCleared),
    })
}

/// /stop — Signal to stop current streaming.
pub fn handle_stop() -> CommandResult {
    CommandResult {
        content: "Stopping current reply...".into(),
        action: Some(CommandAction::StopStream),
    }
}

/// /rename <title> — Rename current session.
pub fn handle_rename(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session to rename")?;
    let title = args.trim();
    if title.is_empty() {
        return Err("Usage: /rename <title>".into());
    }
    session_db
        .update_session_title(sid, title)
        .map_err(|e| e.to_string())?;
    Ok(CommandResult {
        content: format!("Session renamed to **{}**", title),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /sessions [query] — picker of user-conversation sessions, filtering out
/// cron-driven, subagent-child, and incognito sessions (see
/// `SessionMeta.is_regular_chat` for the policy rationale). When `args` is
/// non-empty it acts as a case-insensitive filter that matches sessions if
/// **either** the session metadata (title, short id, agent label, project
/// label, IM channel label) **or** any message body (FTS5) contains the
/// query — important for IM users who can't visually scroll a sidebar nor
/// open Cmd+F.
pub fn handle_sessions(
    session_db: &Arc<SessionDB>,
    args: &str,
) -> Result<CommandResult, String> {
    let query = args.trim();
    let all = session_db.list_sessions(None).map_err(|e| e.to_string())?;

    // Friendly agent display names; one IO per /sessions, fall back to bare
    // ids on error. `list_agents` also reads memory counts we discard —
    // acceptable since /sessions is user-initiated.
    let agent_names: std::collections::HashMap<String, String> =
        crate::agent_loader::list_agents()
            .unwrap_or_default()
            .into_iter()
            .map(|a| (a.id, a.name))
            .collect();
    let project_db = crate::globals::get_project_db();

    // No-query path caps before building picker items; query path keeps the
    // full set so search reaches beyond the most-recent 30.
    let mut filtered = all
        .into_iter()
        .filter(|s| !s.is_cron && s.parent_session_id.is_none());
    let candidates: Vec<crate::session::SessionMeta> = if query.is_empty() {
        filtered.by_ref().take(SESSION_PICKER_LIMIT).collect()
    } else {
        filtered.collect()
    };

    // ProjectDB.get hits SQLite each call; cache by id (including misses).
    let mut project_label_cache: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();

    let mut picker_items: Vec<SessionPickerItem> = candidates
        .iter()
        .map(|s| {
            build_picker_item(s, &agent_names, project_db, &mut project_label_cache)
        })
        .collect();

    if !query.is_empty() {
        apply_query_filter(&mut picker_items, query, session_db);
    }

    let total = picker_items.len();
    picker_items.truncate(SESSION_PICKER_LIMIT);

    let content = render_picker_content(&picker_items, query, total);
    Ok(CommandResult {
        content,
        action: Some(CommandAction::ShowSessionPicker {
            sessions: picker_items,
        }),
    })
}

/// Apply `/sessions <query>` filter: metadata substring ∪ FTS5 message-body
/// match. `picker_items` covers the non-cron / non-subagent / non-incognito
/// candidate set, so retain over that set automatically drops cron /
/// subagent FTS hits.
fn apply_query_filter(
    picker_items: &mut Vec<SessionPickerItem>,
    query: &str,
    session_db: &Arc<SessionDB>,
) {
    // Lowercase fields once per session; otherwise we'd re-allocate them
    // for each haystack on every retain visit.
    let needle = query.to_lowercase();
    let metadata_hits: std::collections::HashSet<String> = picker_items
        .iter()
        .filter(|s| session_matches_query(s, &needle))
        .map(|s| s.id.clone())
        .collect();

    // search_distinct_session_snippets dedupes at the SQL level so a single
    // chatty session can't crowd out other matching sessions inside the
    // FTS limit.
    let fts_pairs = session_db
        .search_distinct_session_snippets(query, SESSION_FTS_LIMIT)
        .unwrap_or_default();
    let fts_snippets: std::collections::HashMap<String, String> = fts_pairs
        .into_iter()
        .map(|(sid, raw)| {
            (
                sid,
                crate::session::strip_fts_snippet_sentinels(
                    &raw,
                    SESSION_PICKER_SNIPPET_BYTES,
                ),
            )
        })
        .collect();

    for item in picker_items.iter_mut() {
        if let Some(snippet) = fts_snippets.get(&item.id) {
            item.snippet = Some(snippet.clone());
        }
    }
    picker_items
        .retain(|s| metadata_hits.contains(&s.id) || fts_snippets.contains_key(&s.id));
}

fn render_picker_content(items: &[SessionPickerItem], query: &str, total: usize) -> String {
    if items.is_empty() {
        return if query.is_empty() {
            "No active sessions.".to_string()
        } else {
            format!("No sessions match `{}`.", query)
        };
    }
    let header = if query.is_empty() {
        format!("**Sessions** ({})", total)
    } else {
        format!("**Sessions** matching `{}` ({})", query, total)
    };
    let mut lines = vec![header];
    for s in items.iter().take(10) {
        lines.push(format_session_picker_line(s));
    }
    if total > 10 {
        lines.push(format!("…and {} more", total - 10));
    }
    lines.join("\n")
}

fn build_picker_item(
    s: &crate::session::SessionMeta,
    agent_names: &std::collections::HashMap<String, String>,
    project_db: Option<&Arc<crate::project::ProjectDB>>,
    project_label_cache: &mut std::collections::HashMap<String, Option<String>>,
) -> SessionPickerItem {
    let agent_label = agent_names
        .get(&s.agent_id)
        .filter(|n| !n.is_empty())
        .cloned()
        .unwrap_or_else(|| s.agent_id.clone());
    let project_label = s.project_id.as_deref().and_then(|pid| {
        project_label_cache
            .entry(pid.to_string())
            .or_insert_with(|| {
                project_db
                    .and_then(|db| db.get(pid).ok().flatten())
                    .map(|p| p.display_label())
            })
            .clone()
    });
    SessionPickerItem {
        id: s.id.clone(),
        title: s.title.clone().unwrap_or_else(|| "(untitled)".to_string()),
        agent_id: s.agent_id.clone(),
        agent_label,
        project_id: s.project_id.clone(),
        project_label,
        channel_label: s.channel_info.as_ref().map(|c| {
            let chat = c.sender_name.clone().unwrap_or_else(|| c.chat_id.clone());
            format!("{} · {}", c.channel_id, chat)
        }),
        updated_at: s.updated_at.clone(),
        snippet: None,
    }
}

fn session_matches_query(s: &SessionPickerItem, needle_lower: &str) -> bool {
    let id_short: String = s.id.chars().take(8).collect();
    let haystacks: [&str; 5] = [
        &s.title,
        &id_short,
        &s.agent_label,
        s.project_label.as_deref().unwrap_or(""),
        s.channel_label.as_deref().unwrap_or(""),
    ];
    haystacks
        .iter()
        .any(|h| !h.is_empty() && h.to_lowercase().contains(needle_lower))
}

/// Format one session row for the markdown body of `/sessions`. Shared by
/// the slash handler (GUI markdown) and the channel text-fallback so the
/// two surfaces stay aligned. When the session was matched via message-body
/// FTS, a second indented line shows the matched snippet.
pub(crate) fn format_session_picker_line(s: &SessionPickerItem) -> String {
    let id_short: String = s.id.chars().take(8).collect();
    let mut chips: Vec<String> = Vec::with_capacity(3);
    if !s.agent_label.is_empty() {
        chips.push(format!("agent: {}", s.agent_label));
    }
    if let Some(pl) = s.project_label.as_deref() {
        chips.push(format!("project: {}", pl));
    }
    if let Some(cl) = s.channel_label.as_deref() {
        chips.push(cl.to_string());
    }
    let suffix = if chips.is_empty() {
        String::new()
    } else {
        format!(" · _{}_", chips.join(" · "))
    };
    let head = format!("- `{}` · {}{}", id_short, s.title, suffix);
    match s.snippet.as_deref() {
        Some(sn) if !sn.is_empty() => format!("{}\n  > {}", head, sn),
        _ => head,
    }
}

/// /session — show / attach / exit. Sub-actions:
/// - **(no args)** — `info` view of the current session (agent / project /
///   working dir / attached IM chats / primary marker).
/// - **`exit`** — detach the current IM chat from its session.
/// - **`<id>` (any other arg)** — attach the current chat to that session.
pub fn handle_session(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return handle_session_info(session_db, session_id);
    }
    if trimmed.eq_ignore_ascii_case("exit") {
        return Ok(CommandResult {
            content: "Detaching this chat from its session...".into(),
            action: Some(CommandAction::DetachFromSession),
        });
    }

    // Treat the remaining argument as a session id (or unique prefix).
    // Validate the id exists before emitting the action so a typo gets
    // caught at the slash layer rather than blowing up inside
    // `attach_session`. Prefix matching keeps the IM text fallback usable
    // on non-button channels (WeChat / iMessage / IRC / Signal / WhatsApp)
    // where users see the 8-char short id from the picker and type that.
    let resolved = resolve_session_id_or_prefix(session_db, trimmed)?;
    Ok(CommandResult {
        content: format!("Attaching to session `{}`...", resolved),
        action: Some(CommandAction::AttachToSession {
            session_id: resolved,
        }),
    })
}

/// Resolve `arg` to a full session id. Tries exact match first; on miss,
/// looks for sessions whose id starts with `arg`. Errors with a helpful
/// message when the prefix is ambiguous or no match exists.
fn resolve_session_id_or_prefix(session_db: &Arc<SessionDB>, arg: &str) -> Result<String, String> {
    if let Some(meta) = session_db.get_session(arg).map_err(|e| e.to_string())? {
        return Ok(meta.id);
    }
    // Fallback: prefix scan. `list_sessions(None)` already excludes
    // incognito; `/sessions` further filters cron / subagent rows for the
    // picker, but for raw resolution we tolerate any non-incognito session
    // — the user explicitly typed the id, after all.
    let candidates: Vec<crate::session::SessionMeta> = session_db
        .list_sessions(None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|s| s.id.starts_with(arg))
        .collect();
    match candidates.len() {
        0 => Err(format!("Session `{}` not found", arg)),
        1 => Ok(candidates.into_iter().next().unwrap().id),
        n => Err(format!(
            "Prefix `{}` is ambiguous — {} sessions match. Use a longer prefix or full id.",
            arg, n
        )),
    }
}

fn handle_session_info(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session")?;
    let meta = session_db
        .get_session(sid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Session `{}` not found", sid))?;

    let mut lines = vec![
        format!(
            "**Session** `{}`",
            meta.id.chars().take(8).collect::<String>()
        ),
        format!("- Title: {}", meta.title.as_deref().unwrap_or("(untitled)")),
        format!("- Agent: `{}`", meta.agent_id),
    ];
    if let Some(pid) = meta.project_id.as_deref() {
        if let Some(project) =
            crate::globals::get_project_db().and_then(|db| db.get(pid).ok().flatten())
        {
            lines.push(format!("- Project: **{}**", project.name));
        }
    }
    if let Some(wd) = crate::session::effective_session_working_dir(Some(sid)).as_deref() {
        lines.push(format!("- Working dir: `{}`", wd));
    }

    if let Some(channel_db) = crate::globals::get_channel_db() {
        if let Ok(attaches) = channel_db.list_attached(sid) {
            if !attaches.is_empty() {
                lines.push(String::new());
                lines.push("**Attached IM channels**".into());
                lines.extend(super::format_attached_channels_lines(&attaches, false));
            }
        }
    }

    Ok(CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /handover — push the current session to an IM chat. Args expect the
/// shape `<channel_id>:<account_id>:<chat_id>[:<thread_id>]`. With no args,
/// we hint to the user to use the GUI Handover dialog (the slash form is
/// for power users / scripting). Always GUI-only — IM-side handovers go
/// through `/session <id>` from the target chat instead.
pub fn handle_handover(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session to hand over")?;
    let _meta = session_db
        .get_session(sid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Session `{}` not found", sid))?;

    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Err(
            "Usage: /handover <channelId>:<accountId>:<chatId>[:<threadId>] (or use the Handover button in the chat header)".into(),
        );
    }

    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() < 3 || parts.len() > 4 {
        return Err("Usage: /handover <channelId>:<accountId>:<chatId>[:<threadId>]".into());
    }
    let channel_id = parts[0].trim();
    let account_id = parts[1].trim();
    let chat_id = parts[2].trim();
    let thread_id = parts.get(3).map(|s| s.trim().to_string());
    if channel_id.is_empty() || account_id.is_empty() || chat_id.is_empty() {
        return Err("Channel id / account id / chat id may not be empty".into());
    }

    Ok(CommandResult {
        content: format!(
            "Handing session over to `{}` / `{}` / `{}`...",
            channel_id, account_id, chat_id
        ),
        action: Some(CommandAction::HandoverToChannel {
            session_id: sid.to_string(),
            channel_id: channel_id.to_string(),
            account_id: account_id.to_string(),
            chat_id: chat_id.to_string(),
            thread_id,
        }),
    })
}
