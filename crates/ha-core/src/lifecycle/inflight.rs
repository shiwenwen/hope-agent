//! Pre-flight inventory of in-flight work that a restart would interrupt.
//!
//! The model uses this to surface "if you press Yes, these N items will be
//! interrupted" before the user confirms. Best-effort: this scans the
//! sources we already track (active chat turns, running async tool jobs,
//! cron jobs marked `running_at IS NOT NULL`). Channel media uploads are
//! intentionally not tracked here — there's no in-memory registry today
//! and adding one just for this would be churn for marginal value.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InflightKind {
    ChatTurn,
    AsyncJob,
    Cron,
}

impl InflightKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            InflightKind::ChatTurn => "chat_turn",
            InflightKind::AsyncJob => "async_job",
            InflightKind::Cron => "cron",
        }
    }
}

impl std::fmt::Display for InflightKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single in-flight item the user should know about before confirming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InflightItem {
    pub kind: InflightKind,
    /// Short user-facing label. e.g. `"chat turn in session abc12 (streaming)"`.
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InflightSummary {
    pub items: Vec<InflightItem>,
}

impl InflightSummary {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// Inventory in-flight work. Never blocks; reads from already-locked
/// in-memory registries + one cheap SQLite scan per source. Returns an
/// empty summary on every collection failure — a noisy pre-flight that
/// misses a turn is worse than one that occasionally under-reports.
pub fn collect_inflight() -> InflightSummary {
    let mut items: Vec<InflightItem> = Vec::new();

    for snap in crate::chat_engine::active_turn::all_current() {
        items.push(InflightItem {
            kind: InflightKind::ChatTurn,
            label: format!(
                "chat turn in session {} ({:?})",
                short_id(&snap.session_id),
                snap.source
            ),
        });
    }

    if let Some(db) = crate::async_jobs::get_async_jobs_db() {
        match db.list_running() {
            Ok(jobs) => {
                for j in jobs {
                    let sess = j
                        .session_id
                        .as_deref()
                        .map(short_id)
                        .unwrap_or_else(|| "—".to_string());
                    items.push(InflightItem {
                        kind: InflightKind::AsyncJob,
                        label: format!(
                            "async tool job {} ({}, session {})",
                            short_id(&j.job_id),
                            j.tool_name,
                            sess,
                        ),
                    });
                }
            }
            Err(e) => app_warn!(
                "lifecycle",
                "inflight",
                "async_jobs list_running failed during pre-flight: {}",
                e
            ),
        }
    }

    // Cron scheduler is Primary-only, so Secondary processes report zero
    // here even when the Primary's cron is mid-tick — correct: only the
    // Primary tier should ever be restarted on its own behalf.
    if let Some(db) = crate::get_cron_db() {
        match db.list_running_jobs() {
            Ok(jobs) => {
                for j in jobs {
                    items.push(InflightItem {
                        kind: InflightKind::Cron,
                        label: format!("cron job '{}' running ({})", j.name, short_id(&j.id)),
                    });
                }
            }
            Err(e) => app_warn!(
                "lifecycle",
                "inflight",
                "cron list_running_jobs failed during pre-flight: {}",
                e
            ),
        }
    }

    InflightSummary { items }
}

fn short_id(s: &str) -> String {
    let trim = s.trim_start_matches("sess-");
    let trim = trim.trim_start_matches("job-");
    let n = trim.chars().count().min(8);
    trim.chars().take(n).collect()
}
