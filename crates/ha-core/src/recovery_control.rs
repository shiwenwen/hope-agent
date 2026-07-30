//! Ephemeral, session-scoped controls for a visible model-recovery wait.
//!
//! Every wait gets a random id. UI actions must match both session and id, so
//! clicking an old retry card can never affect a newer turn or recovery step.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    SkipWait,
    SwitchModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryControlResult {
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryWaitOutcome {
    Elapsed,
    SkipWait,
    SwitchModel,
    Cancelled,
}

#[derive(Debug)]
struct Signal {
    action: AtomicU8,
    notify: Notify,
}

#[derive(Clone)]
struct PendingWait {
    id: String,
    signal: Arc<Signal>,
}

static RECOVERY_WAITS: OnceLock<Mutex<HashMap<String, PendingWait>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, PendingWait>> {
    RECOVERY_WAITS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn registry_lock() -> std::sync::MutexGuard<'static, HashMap<String, PendingWait>> {
    registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn action_code(action: RecoveryAction) -> u8 {
    match action {
        RecoveryAction::SkipWait => 1,
        RecoveryAction::SwitchModel => 2,
    }
}

fn outcome_from_code(code: u8) -> Option<RecoveryWaitOutcome> {
    match code {
        1 => Some(RecoveryWaitOutcome::SkipWait),
        2 => Some(RecoveryWaitOutcome::SwitchModel),
        _ => None,
    }
}

#[derive(Debug)]
pub struct RecoveryWait {
    session_id: String,
    id: String,
    signal: Arc<Signal>,
}

impl RecoveryWait {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub async fn wait(
        &self,
        duration: Duration,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> RecoveryWaitOutcome {
        if let Some(outcome) = outcome_from_code(self.signal.action.load(Ordering::SeqCst)) {
            return outcome;
        }
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return RecoveryWaitOutcome::Cancelled;
        }

        let notified = self.signal.notify.notified();
        tokio::pin!(notified);
        let sleep = tokio::time::sleep(duration);
        tokio::pin!(sleep);

        if let Some(cancel) = cancel {
            tokio::select! {
                biased;
                _ = wait_for_cancel(cancel) => RecoveryWaitOutcome::Cancelled,
                _ = &mut notified => outcome_from_code(self.signal.action.load(Ordering::SeqCst))
                    .unwrap_or(RecoveryWaitOutcome::Elapsed),
                _ = &mut sleep => RecoveryWaitOutcome::Elapsed,
            }
        } else {
            tokio::select! {
                biased;
                _ = &mut notified => outcome_from_code(self.signal.action.load(Ordering::SeqCst))
                    .unwrap_or(RecoveryWaitOutcome::Elapsed),
                _ = &mut sleep => RecoveryWaitOutcome::Elapsed,
            }
        }
    }
}

impl Drop for RecoveryWait {
    fn drop(&mut self) {
        let mut waits = registry_lock();
        let matches = waits
            .get(&self.session_id)
            .is_some_and(|pending| pending.id == self.id);
        if matches {
            waits.remove(&self.session_id);
        }
    }
}

pub fn register(session_id: &str) -> RecoveryWait {
    let id = uuid::Uuid::new_v4().to_string();
    let signal = Arc::new(Signal {
        action: AtomicU8::new(0),
        notify: Notify::new(),
    });
    registry_lock().insert(
        session_id.to_string(),
        PendingWait {
            id: id.clone(),
            signal: Arc::clone(&signal),
        },
    );
    RecoveryWait {
        session_id: session_id.to_string(),
        id,
        signal,
    }
}

pub fn request(
    session_id: &str,
    recovery_id: &str,
    action: RecoveryAction,
) -> RecoveryControlResult {
    let pending = {
        let waits = registry_lock();
        waits.get(session_id).cloned()
    };
    let Some(pending) = pending else {
        return RecoveryControlResult {
            applied: false,
            reason: Some("no_active_recovery_wait".to_string()),
        };
    };
    if pending.id != recovery_id {
        return RecoveryControlResult {
            applied: false,
            reason: Some("stale_recovery_wait".to_string()),
        };
    }

    if pending
        .signal
        .action
        .compare_exchange(0, action_code(action), Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return RecoveryControlResult {
            applied: false,
            reason: Some("recovery_already_controlled".to_string()),
        };
    }
    pending.signal.notify.notify_one();
    RecoveryControlResult {
        applied: true,
        reason: None,
    }
}

async fn wait_for_cancel(cancel: &Arc<AtomicBool>) {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exact_recovery_id_wakes_wait() {
        let session = format!("recovery-{}", uuid::Uuid::new_v4());
        let wait = register(&session);
        let result = request(&session, wait.id(), RecoveryAction::SkipWait);
        assert!(result.applied);
        assert_eq!(
            wait.wait(Duration::from_secs(60), None).await,
            RecoveryWaitOutcome::SkipWait
        );
    }

    #[tokio::test]
    async fn stale_id_cannot_control_new_wait() {
        let session = format!("recovery-{}", uuid::Uuid::new_v4());
        let old = register(&session);
        let old_id = old.id().to_string();
        let current = register(&session);

        let result = request(&session, &old_id, RecoveryAction::SwitchModel);
        assert!(!result.applied);
        assert_eq!(result.reason.as_deref(), Some("stale_recovery_wait"));
        assert!(request(&session, current.id(), RecoveryAction::SwitchModel).applied);
        assert_eq!(
            current.wait(Duration::from_secs(60), None).await,
            RecoveryWaitOutcome::SwitchModel
        );
    }
}
