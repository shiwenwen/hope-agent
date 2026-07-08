use std::sync::Arc;

use super::{SessionDB, SessionMeta};

/// Populate `SessionMeta.pending_interaction_count` on each session by merging
/// pending tool approvals (in-memory registry) with pending `ask_user` groups
/// (SQLite). Safe to call with an empty slice.
///
/// Runs on the sidebar/session-list hot path, so the SQLite read is offloaded
/// to the blocking pool (`db.run`) rather than pinning a runtime worker.
pub async fn enrich_pending_interactions(
    sessions: &mut [SessionMeta],
    db: &Arc<SessionDB>,
) -> anyhow::Result<()> {
    if sessions.is_empty() {
        return Ok(());
    }
    let approvals = crate::tools::approval::pending_approvals_per_session().await;
    let ask_users = db
        .run(|db| db.count_pending_ask_user_groups_per_session())
        .await?;
    if approvals.is_empty() && ask_users.is_empty() {
        return Ok(());
    }
    for s in sessions {
        let a = approvals.get(&s.id).copied().unwrap_or(0);
        let q = ask_users.get(&s.id).copied().unwrap_or(0);
        s.pending_interaction_count = a + q;
    }
    Ok(())
}
