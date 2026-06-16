//! Concurrency cap for explicitly-backgrounded tool jobs (MISC-5).
//!
//! Each background job ([`super::spawn::spawn_explicit_job`]) holds a dedicated
//! OS thread + current-thread runtime for its whole lifetime. Without a ceiling
//! a model could fire `run_in_background: true` (or run under an
//! `always-background` policy) across many rounds and linearly exhaust threads
//! and memory — there is no human gate under YOLO / `auto_approve_tools`.
//!
//! A process-global counter caps how many such runners are live at once. The
//! cap is `config.async_tools.max_concurrent_jobs` (read fresh on each acquire
//! so a settings change takes effect immediately); `0` means unlimited.
//!
//! The slot is RAII: [`try_acquire_job_slot`] hands back a [`JobSlotGuard`] that
//! the spawning code moves into the runner thread, so the slot is released
//! exactly when that thread exits (success, failure, or runtime-build error) —
//! no manual bookkeeping on the many terminal paths.
//!
//! Scope: this gates the *explicit* background path only. Auto-background
//! transfers ([`super::spawn::dispatch_with_auto_background`]) spawn their
//! worker *before* the detach decision, so the slot-RAII model doesn't fit; they
//! are bounded instead by per-turn tool concurrency and the sync budget.

use std::sync::atomic::{AtomicUsize, Ordering};

static RUNNING_JOBS: AtomicUsize = AtomicUsize::new(0);

/// RAII guard for one background-job concurrency slot. Decrements the live
/// counter on drop. Held by the background runner thread for the job's whole
/// lifetime, so the slot frees exactly when the runner exits.
#[derive(Debug)]
pub struct JobSlotGuard {
    /// `false` in unlimited mode (cap == 0): no slot was counted, so drop is a
    /// no-op. `true` when a real slot was reserved and must be released.
    counted: bool,
}

impl Drop for JobSlotGuard {
    fn drop(&mut self) {
        if self.counted {
            RUNNING_JOBS.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Try to reserve a background-job slot. Returns `None` when the configured
/// `max_concurrent_jobs` cap is already saturated; a cap of `0` means unlimited
/// (always `Some`). The reservation is an atomic CAS so concurrent callers can
/// never overshoot the cap.
pub fn try_acquire_job_slot() -> Option<JobSlotGuard> {
    let max = crate::config::cached_config()
        .async_tools
        .max_concurrent_jobs;
    if max == 0 {
        return Some(JobSlotGuard { counted: false });
    }
    let mut current = RUNNING_JOBS.load(Ordering::SeqCst);
    loop {
        if current >= max {
            return None;
        }
        match RUNNING_JOBS.compare_exchange_weak(
            current,
            current + 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return Some(JobSlotGuard { counted: true }),
            Err(actual) => current = actual,
        }
    }
}

/// Number of background-job slots currently reserved. Test-only for now; the
/// production cap path needs only the acquire/release pair.
#[cfg(test)]
pub(crate) fn running_job_slots() -> usize {
    RUNNING_JOBS.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// Serializes these tests — they all read/write the process-global
    /// `RUNNING_JOBS`, so parallel execution would interleave their counts.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn guard_releases_slot_on_drop() {
        let _serial = serial();
        RUNNING_JOBS.store(0, Ordering::SeqCst);
        assert_eq!(running_job_slots(), 0);
        {
            let _g = JobSlotGuard { counted: true };
            RUNNING_JOBS.fetch_add(1, Ordering::SeqCst);
            assert_eq!(running_job_slots(), 1);
        }
        assert_eq!(running_job_slots(), 0);
    }

    #[test]
    fn unlimited_guard_drop_is_noop() {
        let _serial = serial();
        RUNNING_JOBS.store(3, Ordering::SeqCst);
        {
            // counted=false mirrors the cap==0 unlimited path.
            let _g = JobSlotGuard { counted: false };
        }
        // Unchanged: the unlimited guard must not touch the counter.
        assert_eq!(running_job_slots(), 3);
        RUNNING_JOBS.store(0, Ordering::SeqCst);
    }
}
