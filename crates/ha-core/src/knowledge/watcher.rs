//! Filesystem watcher for KB roots (design D6 — production-grade).
//!
//! `notify` (FSEvents / inotify / ReadDirectoryChangesW) feeds a per-KB debounce
//! thread that reconciles the index after the dust settles. This absorbs the
//! noise that external editors / sync tools produce — temp-file + atomic-rename
//! saves, batch rewrites, half-written files — because the reconcile is
//! mtime-skipping (unchanged files cost nothing) and only runs after a quiet
//! window. Our own internal writes already reindex synchronously; a watcher
//! callback for them is a cheap no-op (mtime already current).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Quiet window before a burst of FS events triggers one reconcile.
const DEBOUNCE: Duration = Duration::from_millis(800);
/// Periodic wake so the debounce thread can observe its stop flag.
const POLL: Duration = Duration::from_secs(30);

struct KbWatcher {
    /// Held to keep the OS watch alive; dropping it stops the watch and
    /// disconnects the debounce channel.
    _watcher: RecommendedWatcher,
    stop: Arc<AtomicBool>,
}

fn watchers() -> &'static Mutex<HashMap<String, KbWatcher>> {
    static WATCHERS: OnceLock<Mutex<HashMap<String, KbWatcher>>> = OnceLock::new();
    WATCHERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Start watching a KB's root for external edits. Idempotent per KB.
pub fn start_watcher(kb_id: &str) -> Result<()> {
    {
        let map = watchers().lock().unwrap();
        if map.contains_key(kb_id) {
            return Ok(());
        }
    }

    let root = super::resolve_kb_dir(kb_id)?.dir;
    let root = root.canonicalize().unwrap_or(root);

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if event_relevant(&event) {
                let _ = tx.send(());
            }
        }
    })?;
    watcher.watch(&root, RecursiveMode::Recursive)?;

    let stop = Arc::new(AtomicBool::new(false));
    let kb = kb_id.to_string();
    let stop_thread = stop.clone();
    std::thread::Builder::new()
        .name(format!("kb-watch-{kb}"))
        .spawn(move || debounce_loop(kb, rx, stop_thread))
        .ok();

    watchers().lock().unwrap().insert(
        kb_id.to_string(),
        KbWatcher {
            _watcher: watcher,
            stop,
        },
    );
    crate::app_info!(
        "knowledge",
        "watcher",
        "watching kb {} at {}",
        kb_id,
        root.display()
    );
    Ok(())
}

/// Stop watching a KB (e.g. on delete). No-op if not watching.
pub fn stop_watcher(kb_id: &str) {
    if let Some(w) = watchers().lock().unwrap().remove(kb_id) {
        w.stop.store(true, Ordering::Relaxed);
        // Dropping `_watcher` disconnects the channel; the debounce thread exits.
    }
}

/// Start watchers for every registered KB. Called once at startup (Primary).
pub fn start_all_watchers() {
    let Some(registry) = crate::get_knowledge_db() else {
        return;
    };
    for id in registry.list_all_ids().unwrap_or_default() {
        if let Err(e) = start_watcher(&id) {
            crate::app_warn!(
                "knowledge",
                "watcher",
                "start watcher for kb {} failed: {}",
                id,
                e
            );
        }
    }
}

fn debounce_loop(kb: String, rx: std::sync::mpsc::Receiver<()>, stop: Arc<AtomicBool>) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match rx.recv_timeout(POLL) {
            Ok(()) => {
                // Coalesce the burst: keep draining until quiet for DEBOUNCE.
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
                match super::index::reindex_kb(&kb, false) {
                    Ok(report) if report.changed > 0 || report.removed > 0 => {
                        crate::app_info!(
                            "knowledge",
                            "watcher",
                            "kb {} reconciled: {} changed, {} removed",
                            kb,
                            report.changed,
                            report.removed
                        );
                        if let Some(bus) = crate::get_event_bus() {
                            bus.emit(
                                "knowledge:changed",
                                serde_json::json!({ "kbId": kb, "op": "reindex" }),
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        crate::app_warn!(
                            "knowledge",
                            "watcher",
                            "kb {} reconcile failed: {}",
                            kb,
                            e
                        )
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Only react to markdown file events (skip editor temp noise, dotfiles, dirs).
fn event_relevant(event: &notify::Event) -> bool {
    use notify::EventKind;
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    event.paths.iter().any(|p| is_markdown_path(p))
}

fn is_markdown_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
        .unwrap_or(false)
}
