//! Collect peer-session candidates and render them into a snapshot.

use anyhow::Result;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::config::AwarenessConfig;
use super::types::{ActivityState, AwarenessEntry, AwarenessSnapshot, SessionKind};
use crate::recap::types::SessionFacet;
use crate::session::{SessionDB, SessionMeta};

/// Collect candidate entries for the current session. Never triggers LLM calls.
///
/// `current_agent_id` is used when `cfg.same_agent_only` is true to restrict
/// candidates to the same agent. Pass `None` to skip agent filtering.
pub fn collect_entries(
    db: &SessionDB,
    cfg: &AwarenessConfig,
    current_session_id: &str,
    current_agent_id: Option<&str>,
) -> Result<AwarenessSnapshot> {
    // When same_agent_only is on, push the filter down to SQL so we don't
    // pull all sessions then discard most of them client-side.
    let agent_filter: Option<&str> = if cfg.same_agent_only {
        current_agent_id
    } else {
        None
    };
    let pull_limit = (cfg.max_sessions as u32).saturating_mul(4).max(20);
    let (all_sessions, _total): (Vec<SessionMeta>, u32) = db.list_sessions_paged(
        agent_filter,
        crate::session::ProjectFilter::All,
        Some(pull_limit),
        Some(0),
        None,
    )?;

    // Time cutoff for the lookback window.
    let now = Utc::now();
    let cutoff_dt = now - chrono::Duration::hours(cfg.lookback_hours.max(1));

    // Active session IDs from in-memory registry.
    let active_cutoff = Instant::now()
        .checked_sub(Duration::from_secs(cfg.active_window_secs))
        .unwrap_or_else(Instant::now);
    let active_ids: HashSet<String> = super::registry::active_since(active_cutoff)
        .into_iter()
        .collect();

    // Lazily-cached RecapDb connection. If the first open failed (e.g. file
    // lock), retry on subsequent calls so we don't permanently degrade.
    static RECAP_DB: Lazy<Mutex<Option<crate::recap::db::RecapDb>>> =
        Lazy::new(|| Mutex::new(crate::recap::db::RecapDb::open_default().ok()));
    let mut recap_lock = RECAP_DB.lock().unwrap_or_else(|e| e.into_inner());
    if recap_lock.is_none() {
        *recap_lock = crate::recap::db::RecapDb::open_default().ok();
    }

    // Build candidate entries.
    let mut entries: Vec<AwarenessEntry> = Vec::new();
    let mut active_count = 0usize;

    for meta in all_sessions.into_iter() {
        if entries.len() >= cfg.max_sessions {
            break;
        }
        // Hard exclude current session.
        if meta.id == current_session_id {
            continue;
        }
        // Agent filter.
        if cfg.same_agent_only {
            if let Some(my_agent) = current_agent_id {
                if meta.agent_id != my_agent {
                    continue;
                }
            }
        }
        // Type exclusions.
        let kind = classify_session(&meta);
        if cfg.exclude_cron && matches!(kind, SessionKind::Cron) {
            continue;
        }
        if cfg.exclude_channel && matches!(kind, SessionKind::Channel) {
            continue;
        }
        if cfg.exclude_subagents && matches!(kind, SessionKind::Subagent) {
            continue;
        }

        // Lookback window.
        let updated_at_dt = parse_utc(&meta.updated_at);
        if let Some(dt) = updated_at_dt {
            if dt < cutoff_dt {
                continue;
            }
        }

        let age_secs = updated_at_dt
            .map(|dt| (now - dt).num_seconds().max(0))
            .unwrap_or(i64::MAX);

        let is_active = active_ids.contains(&meta.id);
        let activity = if is_active {
            active_count += 1;
            ActivityState::Active
        } else if age_secs < 3600 {
            ActivityState::Recent
        } else {
            ActivityState::Older
        };

        // Facet enrichment.
        let facet: Option<SessionFacet> = recap_lock
            .as_ref()
            .and_then(|r| r.get_latest_facet(&meta.id).ok().flatten());

        // Fallback preview if no facet.
        let fallback_preview = if facet.is_none() {
            db.last_user_message_preview(&meta.id, cfg.preview_chars)
                .ok()
                .flatten()
        } else {
            None
        };

        let title = meta
            .title
            .clone()
            .filter(|t: &String| !t.trim().is_empty())
            .unwrap_or_else(|| format!("Session {}", crate::truncate_utf8(&meta.id, 8)));

        let agent_name = resolve_agent_name(&meta.agent_id);

        let brief_summary = facet.as_ref().map(|f| f.brief_summary.clone());
        let underlying_goal = facet.as_ref().map(|f| f.underlying_goal.clone());
        let outcome = facet
            .as_ref()
            .map(|f| format!("{:?}", f.outcome).to_lowercase());
        let goal_categories = facet
            .as_ref()
            .map(|f| f.goal_categories.clone())
            .unwrap_or_default();

        entries.push(AwarenessEntry {
            session_id: meta.id.clone(),
            title,
            agent_id: meta.agent_id.clone(),
            agent_name,
            session_kind: kind,
            updated_at: meta.updated_at.clone(),
            age_secs,
            activity,
            brief_summary,
            underlying_goal,
            outcome,
            goal_categories,
            fallback_preview,
        });
    }

    // Sort: Active first, then Recent, then Older; within each group newer first.
    entries.sort_by(|a, b| {
        let rank_a = activity_rank(&a.activity);
        let rank_b = activity_rank(&b.activity);
        rank_a.cmp(&rank_b).then(a.age_secs.cmp(&b.age_secs))
    });

    Ok(AwarenessSnapshot {
        entries,
        active_count,
        generated_at: now.to_rfc3339(),
    })
}

fn activity_rank(a: &ActivityState) -> u8 {
    match a {
        ActivityState::Active => 0,
        ActivityState::Recent => 1,
        ActivityState::Older => 2,
    }
}

fn parse_utc(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn classify_session(meta: &SessionMeta) -> SessionKind {
    if meta.is_cron {
        return SessionKind::Cron;
    }
    if meta.channel_info.is_some() {
        return SessionKind::Channel;
    }
    if meta.parent_session_id.is_some() {
        return SessionKind::Subagent;
    }
    SessionKind::Regular
}

/// Thread-local-ish cache for agent name resolution. Avoids hitting disk
/// for the same agent_id multiple times per snapshot.
static AGENT_NAME_CACHE: Lazy<Mutex<HashMap<String, Option<String>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn resolve_agent_name(agent_id: &str) -> Option<String> {
    let mut cache = AGENT_NAME_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = cache.get(agent_id) {
        return cached.clone();
    }
    let name = crate::agent_loader::load_agent(agent_id)
        .ok()
        .map(|def| def.config.name.clone());
    cache.insert(agent_id.to_string(), name.clone());
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_meta_regular(id: &str) -> SessionMeta {
        SessionMeta {
            id: id.into(),
            title: None,
            title_source: crate::session_title::TITLE_SOURCE_MANUAL.into(),
            agent_id: "default".into(),
            provider_id: None,
            provider_name: None,
            model_id: None,
            created_at: "2025-01-01T00:00:00Z".into(),
            updated_at: "2025-01-01T00:00:00Z".into(),
            message_count: 0,
            unread_count: 0,
            has_error: false,
            pending_interaction_count: 0,
            is_cron: false,
            parent_session_id: None,
            plan_mode: crate::plan::PlanModeState::Off,
            permission_mode: crate::permission::SessionMode::Default,
            channel_info: None,
            project_id: None,
            incognito: false,
            working_dir: None,
        }
    }

    #[test]
    fn classify_cron_session() {
        let mut meta = mk_meta_regular("s1");
        meta.is_cron = true;
        assert_eq!(classify_session(&meta), SessionKind::Cron);
    }

    #[test]
    fn classify_subagent_session() {
        let mut meta = mk_meta_regular("s2");
        meta.parent_session_id = Some("parent".into());
        assert_eq!(classify_session(&meta), SessionKind::Subagent);
    }

    #[test]
    fn classify_regular_session() {
        let meta = mk_meta_regular("s3");
        assert_eq!(classify_session(&meta), SessionKind::Regular);
    }
}
