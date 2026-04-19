//! Per-job-id completion wait registry used by `job_status(block=true)`.
//!
//! Producers ([`crate::async_jobs::spawn::finalize_job`]) call
//! [`notify_completion`] after the DB row reaches a terminal state; consumers
//! ([`crate::tools::job_status::tool_job_status`]) call [`register_waiter`]
//! and park on `Notify::notified()`. The returned `Arc<Notify>` must be held
//! **only for the lifetime of a single `tool_job_status` invocation** —
//! cleanup relies on `Arc::strong_count` semantics under the registry lock.
//!
//! # Lifecycle invariants
//!
//! 1. Entries are only inserted by [`register_waiter`]; eager insertion at job
//!    creation time is avoided so jobs nobody polls don't leak registry slots.
//! 2. Entries are only removed by [`notify_completion`] (producer side) and
//!    [`cleanup_if_last_waiter`] (waiter-side safety net for the
//!    "timed out while job still running" case).
//! 3. `notify_completion` performs `notify_waiters()` and `map.remove(job_id)`
//!    inside the same critical section. This removes the Notify from the map
//!    before any late waiter can observe a stale, already-fired handle.
//!    (`Notify::notify_waiters` sets no permit for future `notified()` calls.)
//! 4. A late waiter that arrives after `notify_completion` inserts a **fresh**
//!    Notify via `register_waiter`. The mandatory post-register DB recheck in
//!    `tool_job_status` then sees the terminal row and returns without ever
//!    parking; the orphan Notify is cleaned up on the waiter's return path.
//!
//! # EventBus coexistence
//!
//! [`crate::async_jobs::spawn::finalize_job`] still emits the
//! `async_tool_job:completed` EventBus event. `job_status` no longer consumes
//! it — the broadcast is kept solely so future frontend UI can subscribe.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::Notify;

static WAITERS: LazyLock<Mutex<HashMap<String, Arc<Notify>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Lazily create (or clone the existing) `Arc<Notify>` for `job_id` and
/// return it to the caller. The caller must park on `notified()` and
/// must drop the returned `Arc` when the `tool_job_status` invocation
/// returns — never store it elsewhere.
pub fn register_waiter(job_id: &str) -> Arc<Notify> {
    let mut map = WAITERS.lock().unwrap_or_else(|p| p.into_inner());
    map.entry(job_id.to_string())
        .or_insert_with(|| Arc::new(Notify::new()))
        .clone()
}

/// Producer-side wake-up. Called from `finalize_job` **after**
/// `db.update_terminal` commits so any awakened waiter sees the terminal
/// row on its post-wake DB reload. Idempotent — calling without a
/// registered waiter is a no-op.
pub fn notify_completion(job_id: &str) {
    let mut map = WAITERS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(notify) = map.remove(job_id) {
        notify.notify_waiters();
    }
}

/// Waiter-side cleanup. Called on the `tool_job_status` return path
/// (terminal, timeout, or error) to remove the registry entry *only* when
/// the caller is the last holder. Other waiters still parked on their own
/// `Notified` futures keep the entry alive.
///
/// Safe because we hold the registry lock across the `strong_count` check
/// and the `remove`: no concurrent `register_waiter` / `notify_completion`
/// can interleave and mutate the count.
pub fn cleanup_if_last_waiter(job_id: &str, my_arc: &Arc<Notify>) {
    let mut map = WAITERS.lock().unwrap_or_else(|p| p.into_inner());
    let should_remove = match map.get(job_id) {
        // One ref in the map + one ref held by the caller => we are the last.
        Some(existing) if Arc::strong_count(existing) <= 2 => Arc::ptr_eq(existing, my_arc),
        _ => false,
    };
    if should_remove {
        map.remove(job_id);
    }
}

#[cfg(test)]
pub fn waiter_count(job_id: &str) -> usize {
    let map = WAITERS.lock().unwrap_or_else(|p| p.into_inner());
    map.get(job_id).map(|n| Arc::strong_count(n)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    fn fresh_id() -> String {
        format!("test_job_{}", uuid::Uuid::new_v4().simple())
    }

    #[tokio::test]
    async fn register_then_notify_wakes_waiter() {
        let job_id = fresh_id();
        let notify = register_waiter(&job_id);

        let task_id = job_id.clone();
        let handle = tokio::spawn(async move {
            let n = register_waiter(&task_id);
            n.notified().await;
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        notify_completion(&job_id);

        timeout(Duration::from_millis(200), handle)
            .await
            .expect("waiter should wake within 200ms")
            .expect("task should not panic");

        drop(notify);
        assert_eq!(waiter_count(&job_id), 0, "entry should be removed");
    }

    #[tokio::test]
    async fn multi_waiter_all_wake() {
        let job_id = fresh_id();
        let n1 = register_waiter(&job_id);
        let n2 = register_waiter(&job_id);
        let n3 = register_waiter(&job_id);

        assert!(Arc::ptr_eq(&n1, &n2));
        assert!(Arc::ptr_eq(&n2, &n3));

        let h1 = {
            let n = n1.clone();
            tokio::spawn(async move { n.notified().await })
        };
        let h2 = {
            let n = n2.clone();
            tokio::spawn(async move { n.notified().await })
        };
        let h3 = {
            let n = n3.clone();
            tokio::spawn(async move { n.notified().await })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        notify_completion(&job_id);

        for h in [h1, h2, h3] {
            timeout(Duration::from_millis(200), h)
                .await
                .expect("all waiters must wake")
                .expect("task should not panic");
        }
    }

    #[tokio::test]
    async fn notify_without_waiter_is_noop() {
        let job_id = fresh_id();
        notify_completion(&job_id);
        assert_eq!(waiter_count(&job_id), 0);
    }

    #[tokio::test]
    async fn late_waiter_after_notify_gets_fresh_entry() {
        let job_id = fresh_id();
        let first = register_waiter(&job_id);
        notify_completion(&job_id);

        let second = register_waiter(&job_id);
        assert!(
            !Arc::ptr_eq(&first, &second),
            "remove-on-notify must give late waiters a fresh Notify"
        );

        cleanup_if_last_waiter(&job_id, &second);
        drop(second);
        drop(first);
        assert_eq!(waiter_count(&job_id), 0);
    }

    #[tokio::test]
    async fn cleanup_if_last_waiter_respects_strong_count() {
        let job_id = fresh_id();
        let a = register_waiter(&job_id);
        let b = register_waiter(&job_id);

        // strong_count >= 3 (map + a + b) → first cleanup keeps the entry.
        cleanup_if_last_waiter(&job_id, &a);
        assert!(
            waiter_count(&job_id) >= 2,
            "entry must survive while another waiter still holds it"
        );

        drop(a);
        // Now only map + b hold refs (count == 2) → next cleanup removes it.
        cleanup_if_last_waiter(&job_id, &b);
        drop(b);
        assert_eq!(waiter_count(&job_id), 0);
    }
}
