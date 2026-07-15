//! 绑定代码目录的文件监听（code→design 回灌的实时触发）。
//!
//! 蓝本 [`crate::knowledge::watcher`]，但两点分歧：① **只 watch 已收割「产物落地文件」的父目录**
//! （`NonRecursive`），不递归仓库根——避免 `node_modules`/`target` 事件洪泛与 Linux inotify 配额；
//! ② 事件按**关联绝对路径集**精确过滤。索引变化（收割/同步/建回执/删产物/绑定变更）后经
//! [`refresh_all`] 全量重建（简单可靠）。父目录整体被删 → watch 静默失效，由打开项目时的
//! 确定性 `check_code_drift` 兜底。监听器进程本地、幂等。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

const DEBOUNCE: Duration = Duration::from_millis(800);
const POLL: Duration = Duration::from_secs(30);

struct DirWatcher {
    _watcher: RecommendedWatcher,
    stop: Arc<AtomicBool>,
}

fn watchers() -> &'static Mutex<HashMap<String, DirWatcher>> {
    static WATCHERS: OnceLock<Mutex<HashMap<String, DirWatcher>>> = OnceLock::new();
    WATCHERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 启动期挂载全部有落地关联的绑定目录（Primary，`app_init`）。
pub fn start_all_watchers() {
    refresh_all();
}

/// 从 DB 全量重建监听器：为每个「有 links 的 code_dir」建/换一个 watcher，撤销已无关联的目录。
pub(crate) fn refresh_all() {
    let Ok(db) = super::service::get_design_db() else {
        return;
    };
    let dirs = match db.list_linked_dirs() {
        Ok(d) => d,
        Err(e) => {
            crate::app_warn!("design", "code_watcher", "list linked dirs failed: {}", e);
            return;
        }
    };
    let wanted: HashSet<String> = dirs.iter().cloned().collect();

    // 撤销不再关联的目录。
    let stale: Vec<String> = {
        let map = watchers().lock().unwrap();
        map.keys()
            .filter(|k| !wanted.contains(*k))
            .cloned()
            .collect()
    };
    for dir in stale {
        stop_watcher(&dir);
    }

    // 为每个目标目录（重）建 watcher，携带其关联绝对路径快照。
    for dir in dirs {
        let abs_paths: HashSet<PathBuf> = match db.links_index_for_dir(&dir) {
            Ok(idx) => idx
                .into_iter()
                .map(|(_proj, _art, rel)| Path::new(&dir).join(rel))
                .collect(),
            Err(_) => continue,
        };
        if abs_paths.is_empty() {
            stop_watcher(&dir);
            continue;
        }
        if let Err(e) = start_watcher(&dir, abs_paths) {
            crate::app_warn!("design", "code_watcher", "watch {} failed: {}", dir, e);
        }
    }
}

/// (重)建单个目录 watcher：watch 关联文件的父目录集（NonRecursive），闭包按路径集精确过滤。
fn start_watcher(code_dir: &str, abs_paths: HashSet<PathBuf>) -> anyhow::Result<()> {
    // 先停旧的（路径集可能变了），再建新的——refresh_all 全量重建的简单语义。
    stop_watcher(code_dir);

    let watched = Arc::new(abs_paths);
    let watched_cb = watched.clone();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if event_relevant(&event, &watched_cb) {
                let _ = tx.send(());
            }
        }
    })?;

    // 父目录去重后逐个 watch（NonRecursive）。父目录不存在则跳过（best-effort）。
    let parents: HashSet<PathBuf> = watched
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();
    let mut watched_any = false;
    for parent in &parents {
        if watcher.watch(parent, RecursiveMode::NonRecursive).is_ok() {
            watched_any = true;
        }
    }
    if !watched_any {
        return Ok(()); // 全部父目录不可 watch（未挂载等）——不注册，靠打开时确定性检查兜底。
    }

    let stop = Arc::new(AtomicBool::new(false));
    let dir = code_dir.to_string();
    let stop_thread = stop.clone();
    std::thread::Builder::new()
        .name(format!("design-codewatch-{}", short_tag(code_dir)))
        .spawn(move || debounce_loop(dir, rx, stop_thread))
        .ok();

    watchers().lock().unwrap().insert(
        code_dir.to_string(),
        DirWatcher {
            _watcher: watcher,
            stop,
        },
    );
    Ok(())
}

fn stop_watcher(code_dir: &str) {
    if let Some(w) = watchers().lock().unwrap().remove(code_dir) {
        w.stop.store(true, Ordering::Relaxed);
    }
}

fn short_tag(dir: &str) -> String {
    Path::new(dir)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("code")
        .chars()
        .take(24)
        .collect()
}

fn debounce_loop(dir: String, rx: std::sync::mpsc::Receiver<()>, stop: Arc<AtomicBool>) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match rx.recv_timeout(POLL) {
            Ok(()) => {
                loop {
                    match rx.recv_timeout(DEBOUNCE) {
                        Ok(()) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(e) = super::code_sync::check_drift_for_dir(&dir) {
                    crate::app_warn!(
                        "design",
                        "code_watcher",
                        "drift check for {} failed: {}",
                        dir,
                        e
                    );
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// 事件相关：Create/Modify/Remove 且路径命中关联绝对路径集（纯函数，可测）。
fn event_relevant(event: &notify::Event, watched: &HashSet<PathBuf>) -> bool {
    use notify::EventKind;
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    event.paths.iter().any(|p| watched.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, EventKind, ModifyKind};

    fn ev(kind: EventKind, path: &str) -> notify::Event {
        notify::Event {
            kind,
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        }
    }

    #[test]
    fn event_relevant_matches_only_watched_paths() {
        let mut watched = HashSet::new();
        watched.insert(PathBuf::from("/repo/src/Button.tsx"));

        // 命中的修改事件。
        assert!(event_relevant(
            &ev(EventKind::Modify(ModifyKind::Any), "/repo/src/Button.tsx"),
            &watched
        ));
        // 命中的创建事件。
        assert!(event_relevant(
            &ev(EventKind::Create(CreateKind::File), "/repo/src/Button.tsx"),
            &watched
        ));
        // 同父目录但非关联文件 → 忽略（NonRecursive 会收到兄弟事件）。
        assert!(!event_relevant(
            &ev(EventKind::Modify(ModifyKind::Any), "/repo/src/Other.tsx"),
            &watched
        ));
        // 非 Create/Modify/Remove（如 Access）→ 忽略。
        assert!(!event_relevant(
            &ev(
                EventKind::Access(notify::event::AccessKind::Read),
                "/repo/src/Button.tsx"
            ),
            &watched
        ));
    }
}
