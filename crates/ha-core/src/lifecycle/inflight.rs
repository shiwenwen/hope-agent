//! Pre-flight inventory of in-flight work that a restart would interrupt.
//!
//! The model uses this to surface "if you press Yes, these N items will be
//! interrupted" before the user confirms. Best-effort: this scans the
//! sources we already track (active chat turns, running async tool jobs,
//! cron jobs marked `running_at IS NOT NULL`). Channel media uploads are
//! intentionally not tracked here — there's no in-memory registry today
//! and adding one just for this would be churn for marginal value.

use serde::{Deserialize, Serialize};

/// A single in-flight item the user should know about before confirming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InflightItem {
    /// `chat_turn` / `async_job` / `cron`. Stable strings rather than an
    /// enum so future categories don't break old log readers.
    pub kind: String,
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

    // Active chat turns — running / streaming / cancelling turns whose
    // current_exe() is us. `chat_engine::active_turn::all_current` returns
    // a flat snapshot of the in-memory registry.
    for snap in crate::chat_engine::active_turn::all_current() {
        let label = format!(
            "chat turn in session {} ({:?})",
            short_id(&snap.session_id),
            snap.source
        );
        items.push(InflightItem {
            kind: "chat_turn".into(),
            label,
        });
    }

    // Async tool jobs (exec / web_search / image_generate / …) — DB lookup
    // is cheap, no in-memory mirror to consult.
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
                        kind: "async_job".into(),
                        label: format!(
                            "async tool job {} ({}, session {})",
                            short_id(&j.job_id),
                            j.tool_name,
                            sess,
                        ),
                    });
                }
            }
            Err(e) => {
                // Pre-flight is best-effort; surface to the log for agent
                // self-diagnosis but don't fail the whole scan.
                app_warn!(
                    "lifecycle",
                    "inflight",
                    "async_jobs list_running failed during pre-flight: {}",
                    e
                );
            }
        }
    }

    // Cron jobs with non-null `running_at`. Cron scheduler is Primary-only,
    // so Secondary processes report zero here even when the Primary's cron
    // is mid-tick — that's correct: only the Primary tier should ever be
    // restarted on its own behalf.
    if let Some(db) = crate::get_cron_db() {
        match db.list_running_jobs() {
            Ok(jobs) => {
                for j in jobs {
                    items.push(InflightItem {
                        kind: "cron".into(),
                        label: format!("cron job '{}' running ({})", j.name, short_id(&j.id)),
                    });
                }
            }
            Err(e) => {
                app_warn!(
                    "lifecycle",
                    "inflight",
                    "cron list_running_jobs failed during pre-flight: {}",
                    e
                );
            }
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
