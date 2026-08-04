//! Autosave 快照原语：每次 config / user_config 写盘前把旧文件拷进
//! `backups/autosave/`，任何设置改动都可回滚。
//!
//! **为什么住在 config 而不是 backup**：写前快照是 config 写路径的安全网，
//! 必须**无条件**执行——`hope-agent server setup` 与两个 server 入口都在
//! `init_runtime` 之前（或完全不经它）写 config，任何「装配期注册」的钩子
//! 都会在这些路径上静默失效。backup.rs（完整备份/恢复，依赖 memory /
//! event_bus）经再导出保留旧路径。快照失败只 warn，绝不阻塞用户写入。

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::paths;

const MAX_AUTOSAVES: usize = 50;

thread_local! {
    /// Optional reason label set by the caller (e.g. the settings tool) that
    /// describes why the next `save_config` / `save_user_config_to_disk` call
    /// is happening. Consumed — and reset — by the very next snapshot.
    static NEXT_SAVE_REASON: RefCell<Option<SaveReason>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone)]
struct SaveReason {
    /// Settings category being updated (e.g. "theme", "proxy", "user").
    category: String,
    /// Who triggered it: "skill", "ui", "cli", ...
    source: String,
}

/// RAII guard set by callers to label the next `save_*` snapshot.
/// Dropping it clears the label even if the save never happens, so a stale
/// label can't contaminate an unrelated subsequent write.
pub struct SaveReasonGuard {
    _private: (),
}

impl Drop for SaveReasonGuard {
    fn drop(&mut self) {
        NEXT_SAVE_REASON.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

/// Label the next config/user_config save so its autosave snapshot records
/// *why* the change happened. Returns a guard — hold it until after the save.
///
/// Example:
/// ```ignore
/// let _g = backup::scope_save_reason("theme", "skill");
/// config::save_config(&store)?; // snapshot tagged "theme/skill"
/// ```
pub fn scope_save_reason(
    category: impl Into<String>,
    source: impl Into<String>,
) -> SaveReasonGuard {
    NEXT_SAVE_REASON.with(|slot| {
        *slot.borrow_mut() = Some(SaveReason {
            category: category.into(),
            source: source.into(),
        });
    });
    SaveReasonGuard { _private: () }
}

fn take_save_reason() -> SaveReason {
    NEXT_SAVE_REASON
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_else(|| SaveReason {
            category: "unknown".into(),
            source: "unknown".into(),
        })
}

/// Snapshot `src` (if it exists) into `backups/autosave/` before it gets
/// overwritten. `kind` is "config" or "user". Errors are logged but never
/// bubbled up — a failed snapshot must not block a legitimate write.
pub fn snapshot_before_write(src: &Path, kind: &str) {
    if !src.exists() {
        // First-ever save — nothing to snapshot.
        // Still consume the reason so it doesn't leak to an unrelated save.
        let _ = take_save_reason();
        return;
    }
    let dir = match paths::autosave_dir() {
        Ok(d) => d,
        Err(e) => {
            app_warn!("backup", "autosave", "Cannot resolve autosave dir: {}", e);
            let _ = take_save_reason();
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        app_warn!("backup", "autosave", "Cannot create autosave dir: {}", e);
        let _ = take_save_reason();
        return;
    }
    let reason = take_save_reason();
    let ts = chrono::Utc::now()
        .format("%Y-%m-%dT%H-%M-%S-%3f")
        .to_string();
    let safe_cat = sanitize_slug(&reason.category);
    let safe_src = sanitize_slug(&reason.source);
    let filename = format!("{}__{}__{}__{}.json", ts, kind, safe_cat, safe_src);
    let dst = dir.join(&filename);
    if let Err(e) = std::fs::copy(src, &dst) {
        app_warn!(
            "backup",
            "autosave",
            "Failed to snapshot {:?} → {:?}: {}",
            src,
            dst,
            e
        );
        return;
    }
    if let Err(e) = rotate_autosaves(&dir, MAX_AUTOSAVES) {
        app_warn!("backup", "autosave", "Rotation failed: {}", e);
    }
}

fn sanitize_slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn rotate_autosaves(dir: &Path, keep: usize) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json") {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    // Names are timestamp-prefixed, so ascending sort = oldest first.
    entries.sort();
    if entries.len() > keep {
        let drop_count = entries.len() - keep;
        for p in entries.iter().take(drop_count) {
            if let Err(e) = std::fs::remove_file(p) {
                app_warn!(
                    "backup",
                    "autosave",
                    "Failed to drop old autosave {:?}: {}",
                    p,
                    e
                );
            }
        }
    }
    Ok(())
}
