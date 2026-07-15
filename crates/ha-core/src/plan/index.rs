// ── Cross-session Plan Index ─────────────────────────────────────
//
// Read-only enumeration of all plan files on disk, joined with session
// metadata from SessionDB. Powers the global "Plans" view and Dashboard
// plan statistics. Live `PlanModeState` is consulted from the in-memory
// store first, falling back to the persisted `sessions.plan_mode` column.
//
// Plans are stored at `~/.hope-agent/plans/<agent_id>/<session_id>/`
// with the current file matching `plan-{ts}-{nano}.md` and historical
// backups appended as `-v{N}.md`. This module never writes.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::file_io::{extract_plan_title, format_sys_time_rfc3339, list_plan_versions};
use super::store::store;
use super::types::PlanModeState;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanIndexFilter {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    /// One of `off|planning|review|executing|completed`. Empty/None disables.
    pub state: Option<String>,
    /// RFC3339 lower bound on `updated_at`. Empty/None disables.
    pub updated_after: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanIndexEntry {
    pub session_id: String,
    /// First 8 chars of `session_id` — what `@plan:<short>` tokens use.
    pub session_short_id: String,
    pub session_title: Option<String>,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub plan_file_path: String,
    pub state: PlanModeState,
    /// First `#` heading of the plan markdown, when present.
    pub title: Option<String>,
    /// Plan markdown file ctime (RFC3339, local time).
    pub created_at: String,
    /// Plan markdown file mtime (RFC3339, local time). Reflects when the
    /// plan file itself last changed — not when execution finished, since
    /// completed plans typically stop touching the file.
    pub updated_at: String,
    /// `sessions.updated_at` for the owning session (RFC3339). Closer to a
    /// real "last activity" timestamp than `updated_at` for executing /
    /// completed plans where the file no longer changes. `None` for orphans.
    pub session_updated_at: Option<String>,
    /// Includes the current file plus all `-v{N}.md` backups.
    pub version_count: u32,
    pub executing_started_at: Option<String>,
    /// Exact completion time of the latest lifecycle. Preserved after the
    /// user archives the Plan by transitioning to Off.
    pub completed_at: Option<String>,
    /// True iff this plan file has no live session (session row deleted).
    pub orphan: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanMentionResolution {
    pub session_id: String,
    pub agent_id: String,
    pub file_path: String,
    pub version: u32,
    pub title: Option<String>,
}

/// Enumerate every plan file on disk, enriched with session metadata.
pub fn list_all_plans(filter: &PlanIndexFilter) -> Result<Vec<PlanIndexEntry>> {
    let plans_dir = crate::paths::plans_dir()?;
    if !plans_dir.exists() {
        return Ok(Vec::new());
    }

    let session_db = crate::get_session_db();
    let live_states: HashMap<String, PlanModeState> = {
        let store_ref = store();
        store_ref
            .try_read()
            .map(|map| map.iter().map(|(k, v)| (k.clone(), v.state)).collect())
            .unwrap_or_default()
    };
    let state_filter = filter
        .state
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(PlanModeState::from_str);

    let mut entries = Vec::new();
    let agent_dirs = std::fs::read_dir(&plans_dir)?;
    for agent_entry in agent_dirs.flatten() {
        let agent_path = agent_entry.path();
        if !agent_path.is_dir() {
            continue;
        }
        let agent_id = agent_entry.file_name().to_string_lossy().into_owned();
        if let Some(ref want) = filter.agent_id {
            if !want.is_empty() && want != &agent_id {
                continue;
            }
        }

        let session_dirs = match std::fs::read_dir(&agent_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        for session_entry in session_dirs.flatten() {
            let session_path = session_entry.path();
            if !session_path.is_dir() {
                continue;
            }
            let session_id = session_entry.file_name().to_string_lossy().into_owned();
            if let Some(ref want) = filter.session_id {
                if !want.is_empty() && want != &session_id {
                    continue;
                }
            }

            let Some(scan) = scan_session_dir(&session_path) else {
                continue;
            };
            let current_path = scan.current_path;
            let (created_at, updated_at) = scan.timestamps;
            let version_count = scan.total_count;

            let session_meta = session_db
                .as_ref()
                .and_then(|db| db.get_session(&session_id).ok().flatten());
            // Incognito sessions are "burn-on-close" by design (see AGENTS.md
            // "关闭即焚"). The plan file may linger on disk before the purge
            // sweep runs, but it must never surface in the global Plans view
            // or Dashboard stats — both consume this index.
            if session_meta.as_ref().is_some_and(|m| m.incognito) {
                continue;
            }
            let executing_started_at = session_db.as_ref().and_then(|db| {
                db.get_session_plan_executing_started_at(&session_id)
                    .ok()
                    .flatten()
            });
            let completed_at = session_db
                .as_ref()
                .and_then(|db| db.get_session_plan_completed_at(&session_id).ok().flatten());
            let (session_title, project_id, db_state, session_updated_at, orphan) =
                match &session_meta {
                    Some(m) => (
                        m.title.clone(),
                        m.project_id.clone(),
                        m.plan_mode,
                        Some(m.updated_at.clone()),
                        false,
                    ),
                    None => (None, None, PlanModeState::Off, None, true),
                };
            let state = live_states.get(&session_id).copied().unwrap_or(db_state);

            if let Some(s) = state_filter {
                if s != state {
                    continue;
                }
            }
            if let Some(ref want_project) = filter.project_id {
                if !want_project.is_empty() && project_id.as_deref() != Some(want_project.as_str())
                {
                    continue;
                }
            }
            if let Some(ref after) = filter.updated_after {
                if !after.is_empty() && updated_at.as_str() < after.as_str() {
                    continue;
                }
            }

            let title = std::fs::read_to_string(&current_path)
                .ok()
                .and_then(|c| extract_plan_title(&c));

            let session_short_id = session_id.chars().take(8).collect::<String>();
            entries.push(PlanIndexEntry {
                session_id,
                session_short_id,
                session_title,
                agent_id: agent_id.clone(),
                project_id,
                plan_file_path: current_path.to_string_lossy().into_owned(),
                state,
                title,
                created_at,
                updated_at,
                session_updated_at,
                version_count,
                executing_started_at,
                completed_at,
                orphan,
            });
        }
    }

    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(entries)
}

/// Resolve a `@plan:<short_id>:v<version>` mention to a concrete file.
/// `version = 0` selects the current plan; positive values pick a backup.
pub fn resolve_plan_mention(short_id: &str, version: u32) -> Result<PlanMentionResolution> {
    let db = crate::get_session_db().ok_or_else(|| anyhow::anyhow!("Session DB unavailable"))?;
    let matches = db.find_sessions_by_id_prefix(short_id)?;
    if matches.is_empty() {
        anyhow::bail!("No session matches plan short_id {}", short_id);
    }
    if matches.len() > 1 {
        anyhow::bail!(
            "Plan short_id {} is ambiguous ({} matches); use a longer prefix",
            short_id,
            matches.len()
        );
    }
    let (session_id, agent_id) = &matches[0];
    let session_dir = crate::paths::session_plans_dir(agent_id, session_id)?;

    let (file_path, resolved_version) = if version == 0 {
        let current = find_current_plan_file(&session_dir)
            .ok_or_else(|| anyhow::anyhow!("No current plan file for session {}", session_id))?;
        (current, 0)
    } else {
        let versions = list_plan_versions(session_id)?;
        let v = versions
            .iter()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Plan version {} not found for session {}",
                    version,
                    session_id
                )
            })?;
        (PathBuf::from(&v.file_path), v.version)
    };

    let title = std::fs::read_to_string(&file_path)
        .ok()
        .and_then(|c| extract_plan_title(&c));

    Ok(PlanMentionResolution {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        file_path: file_path.to_string_lossy().into_owned(),
        version: resolved_version,
        title,
    })
}

// ── Helpers ─────────────────────────────────────────────────────

struct SessionDirScan {
    current_path: PathBuf,
    /// Total `plan-*.md` files (current + every `-v{N}.md` backup).
    total_count: u32,
    /// `(created_at, updated_at)` in RFC3339, derived from the current file
    /// only — backups carry their own mtimes via `list_plan_versions`.
    timestamps: (String, String),
}

/// Walk a session's plan dir once and collect everything callers need: the
/// latest current file, the backup count, and the current file's timestamps.
/// Returns `None` when the directory holds no current plan file.
fn scan_session_dir(dir: &Path) -> Option<SessionDirScan> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut latest: Option<(String, PathBuf, std::fs::Metadata)> = None;
    let mut total_count: u32 = 0;
    for entry in entries.flatten() {
        let Some(name_os) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name_os.starts_with("plan-") || !name_os.ends_with(".md") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        total_count += 1;
        let stem = name_os.trim_end_matches(".md");
        // Skip versioned backups (`{stem}-v{N}.md`) when picking the current.
        let is_backup = stem.rsplit_once("-v").is_some_and(|(_, suffix)| {
            !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
        });
        if is_backup {
            continue;
        }
        if latest
            .as_ref()
            .is_none_or(|(prev, _, _)| name_os.as_str() > prev.as_str())
        {
            latest = Some((name_os, entry.path(), meta));
        }
    }
    let (_, current_path, meta) = latest?;
    let modified = meta
        .modified()
        .ok()
        .map(format_sys_time_rfc3339)
        .unwrap_or_default();
    let created = meta
        .created()
        .ok()
        .map(format_sys_time_rfc3339)
        .unwrap_or_else(|| modified.clone());
    Some(SessionDirScan {
        current_path,
        total_count,
        timestamps: (created, modified),
    })
}

/// Standalone "find latest current plan file" used by `resolve_plan_mention`
/// where we don't need the count / timestamps.
fn find_current_plan_file(dir: &Path) -> Option<PathBuf> {
    scan_session_dir(dir).map(|s| s.current_path)
}
