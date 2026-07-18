use std::sync::Arc;

use super::{PendingCountdown, SessionDB, SessionMeta};
use crate::tools::approval::SessionApprovalAgg;

/// Populate `SessionMeta.pending_interaction_count` and
/// `SessionMeta.pending_countdown` on each session by merging pending tool
/// approvals (in-memory registry) with pending `ask_user` groups (SQLite).
/// Safe to call with an empty slice.
///
/// The countdown is the earliest deadline among pending interactions that
/// carry a timeout; interactions waiting indefinitely don't participate, so a
/// session whose pendings are all timeout-free keeps `pending_countdown: None`
/// and renders exactly as before.
///
/// Runs on the sidebar/session-list hot path, so the SQLite reads are
/// offloaded to the blocking pool (`db.run`) rather than pinning a runtime
/// worker.
pub async fn enrich_pending_interactions(
    sessions: &mut [SessionMeta],
    db: &Arc<SessionDB>,
) -> anyhow::Result<()> {
    if sessions.is_empty() {
        return Ok(());
    }
    let approvals = crate::tools::approval::pending_approvals_per_session().await;
    let (ask_user_counts, ask_user_deadlines) = db
        .run(|db| {
            let counts = db.count_pending_ask_user_groups_per_session()?;
            let deadlines = db.min_pending_ask_user_deadline_per_session()?;
            anyhow::Ok((counts, deadlines))
        })
        .await?;
    if approvals.is_empty() && ask_user_counts.is_empty() {
        return Ok(());
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    for s in sessions {
        let approval_agg = approvals.get(&s.id).copied().unwrap_or_default();
        let q = ask_user_counts.get(&s.id).copied().unwrap_or(0);
        s.pending_interaction_count = approval_agg.count + q;
        s.pending_countdown =
            merge_earliest_countdown(approval_agg, ask_user_deadlines.get(&s.id).copied(), now_ms);
    }
    Ok(())
}

/// Pick the earliest deadline between the approval aggregate (already ms) and
/// the ask_user minimum (`(deadline_secs, created_secs)`, epoch seconds).
/// Returns `None` when neither side has a pending interaction with a timeout.
fn merge_earliest_countdown(
    approval: SessionApprovalAgg,
    ask_user: Option<(i64, i64)>,
    now_ms: i64,
) -> Option<PendingCountdown> {
    let approval_candidate = approval.min_deadline_ms.zip(approval.min_deadline_total_ms);
    let ask_user_candidate = ask_user.and_then(|(deadline_secs, created_secs)| {
        let deadline_ms = deadline_secs.checked_mul(1000)?;
        let total_ms = deadline_secs
            .saturating_sub(created_secs)
            .checked_mul(1000)?;
        Some((deadline_ms, total_ms))
    });
    let earliest = match (approval_candidate, ask_user_candidate) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    };
    earliest.map(|(deadline_at_ms, total_ms)| PendingCountdown {
        deadline_at_ms,
        total_ms: total_ms.max(1),
        server_now_ms: now_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval_agg(deadline_ms: Option<i64>, total_ms: Option<i64>) -> SessionApprovalAgg {
        SessionApprovalAgg {
            count: 1,
            min_deadline_ms: deadline_ms,
            min_deadline_total_ms: total_ms,
        }
    }

    #[test]
    fn no_timeouts_yields_none() {
        assert!(merge_earliest_countdown(SessionApprovalAgg::default(), None, 0).is_none());
    }

    #[test]
    fn approval_only_passes_through_ms() {
        let c = merge_earliest_countdown(approval_agg(Some(120_000), Some(60_000)), None, 500)
            .expect("countdown");
        assert_eq!(
            (c.deadline_at_ms, c.total_ms, c.server_now_ms),
            (120_000, 60_000, 500)
        );
    }

    #[test]
    fn ask_user_only_converts_seconds_to_ms() {
        // deadline 2000s, created 1700s → total 300s.
        let c = merge_earliest_countdown(SessionApprovalAgg::default(), Some((2_000, 1_700)), 0)
            .expect("countdown");
        assert_eq!((c.deadline_at_ms, c.total_ms), (2_000_000, 300_000));
    }

    #[test]
    fn earlier_side_wins_across_sources() {
        // Approval at 90s beats ask_user at 100s…
        let c =
            merge_earliest_countdown(approval_agg(Some(90_000), Some(30_000)), Some((100, 40)), 0)
                .expect("countdown");
        assert_eq!((c.deadline_at_ms, c.total_ms), (90_000, 30_000));
        // …and the other way around, keeping each side's own total.
        let c = merge_earliest_countdown(
            approval_agg(Some(200_000), Some(30_000)),
            Some((100, 40)),
            0,
        )
        .expect("countdown");
        assert_eq!((c.deadline_at_ms, c.total_ms), (100_000, 60_000));
    }

    #[test]
    fn degenerate_total_clamps_to_one() {
        // created == deadline (fallback path) → total clamped to 1ms, never 0.
        let c = merge_earliest_countdown(SessionApprovalAgg::default(), Some((100, 100)), 0)
            .expect("countdown");
        assert_eq!(c.total_ms, 1);
    }
}
