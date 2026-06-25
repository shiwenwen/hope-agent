use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

/// Live cancel flags for jobs that have already registered (i.e. their run has
/// reached `execute_claimed_job` and called [`register`]). **Keyed by `job_id`,
/// but the value carries the run's `claimed_at`** so a cancel request can prove
/// it targets *this* live run and not a later re-claim of a recurring job (see
/// [`cancel`] / [`remove`] — §9 review fix: the live-flag path used to flip
/// whatever run was live, regardless of which run the caller meant).
static CANCELS: LazyLock<Mutex<HashMap<String, (String, Arc<AtomicBool>)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// §9 (C7): pending cancels for jobs claimed (running_at set) but whose run
/// hasn't reached [`register`] yet. **Keyed by the run's `claimed_at`**, not a
/// bare job id: `cancel_running_job` reads `running_at` and then (after a TOCTOU
/// gap) calls [`cancel`], so the in-flight run could finish in between and a
/// later run of a *recurring* job could otherwise inherit the stale placeholder.
/// Recording the claim timestamp means [`register`] only honors a placeholder
/// that targets *this* run (`pending_claimed_at == claimed_at`); a placeholder
/// left by a since-finished run is drained but ignored, so it can never cancel a
/// different run. [`remove`] clears the entry at run end.
static PENDING_CANCELS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Register a cancel flag for a starting run, identified by its `claimed_at`. If
/// a cancel for *this* run arrived during the claim→register window (a pending
/// placeholder keyed to the same `claimed_at`), the flag starts already set so
/// the run is cancelled at its first checkpoint. A placeholder for a different
/// (earlier, since-finished) run is drained but does not set the flag.
pub(crate) fn register(job_id: &str, claimed_at: &str) -> Arc<AtomicBool> {
    let targets_this_run = {
        let mut pending = PENDING_CANCELS.lock().unwrap_or_else(|p| p.into_inner());
        // Always drain (clears stale placeholders); honor only on an exact match.
        match pending.remove(job_id) {
            Some(pending_claimed_at) => pending_claimed_at == claimed_at,
            None => false,
        }
    };
    let flag = Arc::new(AtomicBool::new(targets_this_run));
    {
        let mut map = CANCELS.lock().unwrap_or_else(|p| p.into_inner());
        map.insert(job_id.to_string(), (claimed_at.to_string(), flag.clone()));
    }
    flag
}

/// Request cancellation of the run identified by `claimed_at`. Returns `true` if
/// the request was recorded — either by flipping a live flag, or (during the
/// claim→register window) by leaving a run-keyed pending placeholder that
/// [`register`] will pick up only for the matching run.
pub(crate) fn cancel(job_id: &str, claimed_at: &str) -> bool {
    // C09: only the Primary process claims+registers cron runs, so only there is a
    // claim→register window where a pending placeholder is legitimate. A
    // non-Primary cancel can't reach the run's live flag (it lives in the Primary's
    // memory) — gate the placeholder branch on `is_primary` so a cross-process
    // cancel reports not-cancelled (and leaves no never-drained placeholder)
    // instead of lying `true`; it then falls back to the job timeout (C5).
    cancel_with_pending(job_id, claimed_at, crate::runtime_lock::is_primary())
}

/// Inner cancel with the placeholder branch gated by `allow_pending` (= the caller
/// is the Primary that registers runs). Split out so the run-keyed live-flag logic
/// stays unit-testable with `allow_pending = true` (simulating the Primary).
fn cancel_with_pending(job_id: &str, claimed_at: &str, allow_pending: bool) -> bool {
    {
        let map = CANCELS.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((live_claimed_at, flag)) = map.get(job_id) {
            if live_claimed_at.as_str() == claimed_at {
                flag.store(true, Ordering::SeqCst);
                return true;
            }
            // A *different* run is live now — the run identified by `claimed_at`
            // already finished and a later run of this (recurring) job re-claimed
            // the same job_id. Flipping the live flag would cancel that newer run,
            // which the caller never targeted (the §9 review TOCTOU). The targeted
            // run is gone, so there is nothing to cancel.
            return false;
        }
    }
    // No live flag yet. A pending placeholder is only legitimate in the Primary's
    // claim→register window; a non-Primary process (where the run isn't and never
    // will be) must not leave one (it would never be drained) and must report
    // not-cancelled (C09).
    if !allow_pending {
        return false;
    }
    // Record a run-keyed pending cancel for `register` to drain (honored only on an
    // exact `claimed_at` match, so it can't leak onto a different run either).
    let mut pending = PENDING_CANCELS.lock().unwrap_or_else(|p| p.into_inner());
    pending.insert(job_id.to_string(), claimed_at.to_string());
    true
}

/// Clear a run's cancel state at terminal, **run-keyed by `claimed_at`**. Only
/// drops the live flag / pending placeholder if it still belongs to THIS run: a
/// later run of a recurring job may have re-registered under the same `job_id`
/// between this run clearing `running_at` and its guard dropping, and a blind
/// `remove(job_id)` would clear that newer run's live flag — dropping a
/// concurrent cancel targeting it.
pub(crate) fn remove(job_id: &str, claimed_at: &str) {
    {
        let mut map = CANCELS.lock().unwrap_or_else(|p| p.into_inner());
        if matches!(map.get(job_id), Some((live_at, _)) if live_at.as_str() == claimed_at) {
            map.remove(job_id);
        }
    }
    let mut pending = PENDING_CANCELS.lock().unwrap_or_else(|p| p.into_inner());
    if matches!(pending.get(job_id), Some(p) if p.as_str() == claimed_at) {
        pending.remove(job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_cancel_before_register_starts_run_cancelled() {
        let job = "job-pending";
        // Cancel arrives in the claim→register window for run "ts-1" (Primary).
        assert!(
            cancel_with_pending(job, "ts-1", true),
            "cancel records a pending placeholder"
        );
        // The same run then registers and must observe the cancel immediately.
        let flag = register(job, "ts-1");
        assert!(
            flag.load(Ordering::SeqCst),
            "register drained pending cancel"
        );
        remove(job, "ts-1");
    }

    #[test]
    fn live_cancel_flips_registered_flag() {
        let job = "job-live";
        let flag = register(job, "ts-1");
        assert!(!flag.load(Ordering::SeqCst));
        assert!(cancel(job, "ts-1"));
        assert!(flag.load(Ordering::SeqCst));
        remove(job, "ts-1");
    }

    #[test]
    fn live_flag_for_a_different_run_is_not_cancelled() {
        // §9 review fix: a cancel computed against finished run "ts-1" must NOT
        // flip the live flag of a *later* run "ts-2" that has already registered
        // under the same job_id (recurring job re-claimed in the TOCTOU window).
        let job = "job-recurring-live";
        let flag = register(job, "ts-2"); // the newer run is live
        assert!(
            !cancel(job, "ts-1"),
            "stale cancel for a finished run reports nothing cancelled"
        );
        assert!(
            !flag.load(Ordering::SeqCst),
            "the live run ts-2 must not be cancelled by a stale ts-1 request"
        );
        // And the matching cancel still works on the live run.
        assert!(cancel(job, "ts-2"));
        assert!(flag.load(Ordering::SeqCst));
        remove(job, "ts-2");
    }

    #[test]
    fn stale_pending_for_a_finished_run_does_not_cancel_a_later_run() {
        // Regression guard for the §9 review finding: a cancel that lands after
        // run "ts-1" already finished (its remove() ran) leaves a placeholder
        // keyed to "ts-1"; the NEXT run "ts-2" of a recurring job must NOT be
        // cancelled by it.
        let job = "job-recurring";
        remove(job, "ts-1"); // run ts-1 finished, cleared its state
        assert!(
            cancel_with_pending(job, "ts-1", true),
            "delayed cancel records ts-1 placeholder"
        );
        let flag = register(job, "ts-2"); // a different, later run starts
        assert!(
            !flag.load(Ordering::SeqCst),
            "stale ts-1 placeholder must not cancel run ts-2"
        );
        remove(job, "ts-2");
    }

    #[test]
    fn remove_clears_unconsumed_pending() {
        let job = "job-leak";
        assert!(cancel_with_pending(job, "ts-1", true));
        remove(job, "ts-1");
        let flag = register(job, "ts-1");
        assert!(
            !flag.load(Ordering::SeqCst),
            "remove cleared the placeholder before register could drain it"
        );
        remove(job, "ts-1");
    }

    #[test]
    fn cross_process_cancel_without_live_flag_reports_not_cancelled() {
        // C09: a non-Primary cancel (allow_pending=false) for a run with no local
        // live flag must return false (not-cancelled) and leave NO placeholder —
        // the run is in another process; this falls back to the job timeout (C5).
        let job = "job-cross-process";
        assert!(
            !cancel_with_pending(job, "ts-1", false),
            "cross-process cancel reports not-cancelled"
        );
        // No placeholder was left: a later register here does NOT start cancelled.
        let flag = register(job, "ts-1");
        assert!(
            !flag.load(Ordering::SeqCst),
            "no leaked placeholder from the cross-process cancel"
        );
        remove(job, "ts-1");
    }
}
