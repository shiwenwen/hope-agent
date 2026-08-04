//! Artifact 隐私切换互斥锁（自 artifacts/ 下沉 kernel：incognito 切换
//! （session/db.rs）与 durable Artifact 写入（ha-design 特征侧）必须串行
//! 化，锁必须是两边共享的同一把——kernel 持有，特征侧经原路径再导出）。

use anyhow::{anyhow, Result};

static ARTIFACT_PRIVACY_TRANSITION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn lock_privacy_transition() -> Result<std::sync::MutexGuard<'static, ()>> {
    ARTIFACT_PRIVACY_TRANSITION_LOCK
        .lock()
        .map_err(|_| anyhow!("Artifact privacy transition lock is poisoned"))
}
