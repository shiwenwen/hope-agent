//! Artifact 隐私切换互斥锁（自 artifacts/ 下沉 kernel：incognito 切换
//! （session/db.rs）与 durable Artifact 写入（ha-design 特征侧）必须串行
//! 化，锁必须是两边共享的同一把——kernel 持有，特征侧经原路径再导出）。

use anyhow::{anyhow, Result};

static ARTIFACT_PRIVACY_TRANSITION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const ARTIFACT_PRIVACY_LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// Holds both the process-local ordering mutex and the OS advisory lock shared
/// by Desktop, server, and ACP processes that point at the same data root.
pub struct ArtifactPrivacyTransitionGuard {
    // Drop the OS handle first while the process mutex is still held, so a
    // local waiter cannot briefly race ahead of another process.
    _os_guard: std::fs::File,
    _process_guard: std::sync::MutexGuard<'static, ()>,
}

pub fn lock_privacy_transition() -> Result<ArtifactPrivacyTransitionGuard> {
    let process_guard = ARTIFACT_PRIVACY_TRANSITION_LOCK
        .lock()
        .map_err(|_| anyhow!("Artifact privacy transition lock is poisoned"))?;
    let path = artifact_privacy_lock_path()?;
    loop {
        if let Some(os_guard) = crate::platform::try_acquire_exclusive_lock(&path)? {
            return Ok(ArtifactPrivacyTransitionGuard {
                _os_guard: os_guard,
                _process_guard: process_guard,
            });
        }
        std::thread::sleep(ARTIFACT_PRIVACY_LOCK_POLL);
    }
}

/// Best-effort form used by read paths that may already run under the same
/// non-reentrant mutation lock. `None` means another mutation (or the current
/// caller) owns the lock, so derived projection maintenance must be skipped.
pub fn try_lock_privacy_transition() -> Result<Option<ArtifactPrivacyTransitionGuard>> {
    let process_guard = match ARTIFACT_PRIVACY_TRANSITION_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
        Err(std::sync::TryLockError::Poisoned(_)) => {
            return Err(anyhow!("Artifact privacy transition lock is poisoned"));
        }
    };
    let path = artifact_privacy_lock_path()?;
    let Some(os_guard) = crate::platform::try_acquire_exclusive_lock(&path)? else {
        return Ok(None);
    };
    Ok(Some(ArtifactPrivacyTransitionGuard {
        _os_guard: os_guard,
        _process_guard: process_guard,
    }))
}

fn artifact_privacy_lock_path() -> Result<std::path::PathBuf> {
    Ok(crate::paths::root_dir()?.join(".artifact-privacy-transition.lock"))
}
