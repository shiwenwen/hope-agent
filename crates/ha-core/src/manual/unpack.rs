//! Stable on-disk mirror of the embedded manual for the agent path.
//!
//! The `ha-manual` skill reads/greps real files, so the embedded manual is
//! mirrored to `<data-dir>/manual/{zh,en}/NN.md` (modeled on the browser
//! extension's stable-copy machinery: byte-diff mirror + prune + a sibling
//! marker storing the source-set fingerprint, so a binary upgrade with
//! unchanged docs short-circuits and changed docs re-mirror automatically).
//!
//! Basenames are normalized to ASCII (`04.md`, not `04-记忆系统.md`;
//! `index.md` for the README) so CJK filenames never reach the filesystem —
//! sidestepping Windows non-ASCII quirks and macOS NFD / Linux NFC
//! normalization drift. Cross-chapter links inside the mirrored copies are
//! rewritten to the ASCII names so they stay followable.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;

/// Ensure the on-disk mirror matches the embedded manual, returning the dir.
///
/// Deliberately NOT path-cached for the process: every call re-validates the
/// mirror with a cheap check (fingerprint of the embedded source set — cached
/// per process in release builds — against the sibling marker, plus one stat
/// per expected file), so a mirror deleted fully or partially while the app
/// runs is rebuilt on the next trigger, keeping the documented "safe to
/// delete — rebuilt on next use" contract. The expensive derive + write path
/// runs only when that check misses. Never panics; failures are logged and
/// reported as `None`.
pub fn ensure_local_manual() -> Option<PathBuf> {
    let dir = crate::paths::manual_dir().ok()?;
    let marker = crate::paths::manual_marker().ok()?;
    ensure_local_manual_in(&dir, &marker)
}

fn ensure_local_manual_in(dir: &Path, marker: &Path) -> Option<PathBuf> {
    // Serialize mirrors within the process: the startup background task and
    // the lazy ensure from the Help window / skill activation can race, and
    // two interleaved mirrors prune each other's in-flight temp files.
    //
    // Known limitation (accepted): the lock is process-local. Two long-running
    // binaries with DIFFERENT embedded manual versions sharing one data dir
    // (mixed-version coexistence during an upgrade) each re-mirror to their
    // own fingerprint on their triggers. Per-file writes are atomic so the
    // tree is never torn mid-file; readers can transiently see the other
    // version's text until the stragglers restart.
    static ENSURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let (fingerprint, expected) = source_state();
    if expected.is_empty() {
        crate::app_warn!("manual", "mirror", "no embedded manual files to mirror");
        return None;
    }
    if mirror_is_current(dir, marker, &fingerprint, &expected) {
        return Some(dir.to_path_buf());
    }
    let files = derived_files();
    if files.is_empty() {
        crate::app_warn!("manual", "mirror", "no derivable manual files to mirror");
        return None;
    }
    // Clear the marker first: while the mirror is in progress the copy may be
    // partial, so readers must not trust it until we re-stamp on success.
    let _ = std::fs::remove_file(marker);
    match mirror_files(&files, dir) {
        Ok(()) => {
            if let Err(e) = crate::platform::write_atomic(marker, fingerprint.as_bytes()) {
                crate::app_warn!(
                    "manual",
                    "mirror",
                    "mirrored manual but failed to stamp marker {}: {:#}",
                    marker.display(),
                    e
                );
                return None;
            }
            crate::app_info!(
                "manual",
                "mirror",
                "mirrored {} manual files to {}",
                files.len(),
                dir.display()
            );
            Some(dir.to_path_buf())
        }
        Err(e) => {
            crate::app_warn!(
                "manual",
                "mirror",
                "failed to mirror manual to {}: {:#}",
                dir.display(),
                e
            );
            None
        }
    }
}

/// Fingerprint of the embedded SOURCE set plus the expected mirror file list,
/// both derivable without parsing chapter bodies. Cached per process in
/// release builds (the embed is fixed for the binary's lifetime); recomputed
/// in debug so live doc edits re-mirror on the next trigger.
fn source_state() -> (String, Vec<String>) {
    #[cfg(not(debug_assertions))]
    {
        static STATE: std::sync::OnceLock<(String, Vec<String>)> = std::sync::OnceLock::new();
        STATE.get_or_init(compute_source_state).clone()
    }
    #[cfg(debug_assertions)]
    compute_source_state()
}

fn compute_source_state() -> (String, Vec<String>) {
    let files = super::embed::manual_files();
    let mut hasher = blake3::Hasher::new();
    let mut expected = Vec::new();
    for (rel, bytes) in &files {
        hasher.update(&(rel.len() as u64).to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        let (lang, basename) = match rel.strip_prefix("en/") {
            Some(rest) => ("en", rest),
            None => ("zh", rel.as_str()),
        };
        if basename.contains('/') || !basename.ends_with(".md") {
            continue;
        }
        if let Some(number) = super::model::chapter_number(basename) {
            expected.push(mirror_rel_path(lang, number));
        }
    }
    (hasher.finalize().to_hex()[..16].to_string(), expected)
}

fn mirror_rel_path(lang: &str, number: u8) -> String {
    if number == 0 {
        format!("{lang}/index.md")
    } else {
        format!("{lang}/{number:02}.md")
    }
}

/// The mirror is usable only when the marker matches the current source
/// fingerprint AND every expected file is present — a partially deleted tree
/// (one chapter, or a whole language) re-mirrors instead of lingering broken.
fn mirror_is_current(dir: &Path, marker: &Path, fingerprint: &str, expected: &[String]) -> bool {
    match std::fs::read_to_string(marker) {
        Ok(actual) if actual.trim() == fingerprint => {}
        _ => return false,
    }
    expected.iter().all(|rel| {
        rel.split('/')
            .fold(dir.to_path_buf(), |p, seg| p.join(seg))
            .is_file()
    })
}

/// Derived `(relative path, bytes)` mirror set: `{lang}/{NN}.md` with
/// `index.md` for the README, links rewritten to the ASCII basenames.
fn derived_files() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for lang in ["zh", "en"] {
        for chapter in super::model::chapters(lang) {
            let basename = if chapter.number == 0 {
                "index.md".to_string()
            } else {
                format!("{:02}.md", chapter.number)
            };
            let body = rewrite_links(&chapter.body, lang);
            out.push((format!("{lang}/{basename}"), body.into_bytes()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Rewrite intra-manual link targets to the mirrored ASCII layout:
/// `NN-anything.md` → `NN.md`, `README.md` → `index.md`, and the two
/// cross-language README switch links → `../<lang>/index.md`. Everything
/// else (anchors, external URLs, links escaping the manual) is untouched.
fn rewrite_links(body: &str, lang: &str) -> String {
    static CHAPTER_LINK: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\]\((?:\./)?(\d{2})-[^)#]*?\.md").unwrap());
    let mut out = CHAPTER_LINK.replace_all(body, "]($1.md").into_owned();
    match lang {
        "zh" => {
            out = out.replace("](en/README.md", "](../en/index.md");
        }
        _ => {
            out = out.replace("](../README.md", "](../zh/index.md");
        }
    }
    out.replace("](README.md", "](index.md")
}

/// Byte-diff mirror + prune of entries not in the set (renamed/removed
/// chapters never linger).
fn mirror_files(files: &[(String, Vec<u8>)], dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    let mut keep: HashSet<PathBuf> = HashSet::new();
    for (rel, bytes) in files {
        if rel
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        {
            continue;
        }
        let dest = rel.split('/').fold(dst.to_path_buf(), |p, seg| p.join(seg));
        for ancestor in dest.ancestors().skip(1) {
            if ancestor == dst {
                break;
            }
            keep.insert(ancestor.to_path_buf());
        }
        keep.insert(dest.clone());
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let differs = std::fs::read(&dest)
            .map(|cur| cur != *bytes)
            .unwrap_or(true);
        if differs {
            crate::platform::write_atomic(&dest, bytes)
                .with_context(|| format!("writing {}", dest.display()))?;
        }
    }
    prune_unlisted(dst, &keep)
}

fn prune_unlisted(dir: &Path, keep: &HashSet<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        // Never sweep a write_atomic in-flight temp — it may belong to a
        // concurrent mirror in another process.
        if entry.file_name().to_string_lossy().contains(".tmp.") {
            continue;
        }
        if path.is_dir() {
            if keep.contains(&path) {
                prune_unlisted(&path, keep)?;
            } else {
                std::fs::remove_dir_all(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
        } else if !keep.contains(&path) {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_ascii_layout_and_short_circuits_when_current() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("manual");
        let marker = tmp.path().join(".manual-synced");

        let out = ensure_local_manual_in(&dir, &marker).expect("mirror");
        assert_eq!(out, dir);
        for lang in ["zh", "en"] {
            assert!(dir.join(lang).join("index.md").is_file());
            for n in 1..=13u8 {
                let f = dir.join(lang).join(format!("{n:02}.md"));
                assert!(f.is_file(), "missing {}", f.display());
                // ASCII-only basenames by construction.
                assert!(f.file_name().unwrap().to_str().unwrap().is_ascii());
            }
        }
        // Second run with an unchanged fingerprint must short-circuit: a
        // stray file survives because nothing is pruned.
        let stray = dir.join("zh").join("stray.txt");
        std::fs::write(&stray, b"x").unwrap();
        ensure_local_manual_in(&dir, &marker).expect("short-circuit");
        assert!(stray.is_file(), "short-circuit path should not prune");
        // A stale marker forces a re-mirror, which prunes the stray.
        std::fs::write(&marker, b"stale").unwrap();
        ensure_local_manual_in(&dir, &marker).expect("re-mirror");
        assert!(!stray.exists(), "re-mirror should prune unlisted files");
    }

    /// "Safe to delete — rebuilt on next use": full AND partial deletions
    /// must self-heal on the next ensure, marker intact or not.
    #[test]
    fn deleted_mirror_files_are_rebuilt_on_next_ensure() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("manual");
        let marker = tmp.path().join(".manual-synced");
        ensure_local_manual_in(&dir, &marker).unwrap();

        // Partial deletion: one chapter file gone, marker still matching.
        let victim = dir.join("en").join("05.md");
        std::fs::remove_file(&victim).unwrap();
        ensure_local_manual_in(&dir, &marker).expect("partial re-mirror");
        assert!(victim.is_file(), "deleted chapter file was not rebuilt");

        // Whole-language deletion.
        std::fs::remove_dir_all(dir.join("en")).unwrap();
        ensure_local_manual_in(&dir, &marker).expect("language re-mirror");
        assert!(dir.join("en").join("index.md").is_file());

        // Whole-tree deletion (the documented case).
        std::fs::remove_dir_all(&dir).unwrap();
        ensure_local_manual_in(&dir, &marker).expect("full re-mirror");
        assert!(dir.join("zh").join("01.md").is_file());
    }

    #[test]
    fn mirrored_links_point_at_ascii_names() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("manual");
        let marker = tmp.path().join(".manual-synced");
        ensure_local_manual_in(&dir, &marker).unwrap();

        let zh_index = std::fs::read_to_string(dir.join("zh/index.md")).unwrap();
        assert!(zh_index.contains("](01.md"), "chapter links not rewritten");
        assert!(
            !Regex::new(r"\]\(\d{2}-").unwrap().is_match(&zh_index),
            "original chapter filenames leaked into the mirror"
        );
        assert!(
            zh_index.contains("](../en/index.md"),
            "language switch link"
        );

        let en_index = std::fs::read_to_string(dir.join("en/index.md")).unwrap();
        assert!(
            en_index.contains("](../zh/index.md"),
            "reverse language link"
        );
    }

    #[test]
    fn rewrite_links_shapes() {
        let zh = rewrite_links(
            "见 [02 模型](02-模型与Provider.md#锚点) 与 [目录](README.md)、[English](en/README.md)",
            "zh",
        );
        assert_eq!(
            zh,
            "见 [02 模型](02.md#锚点) 与 [目录](index.md)、[English](../en/index.md)"
        );
        let en = rewrite_links(
            "See [02](02-models-and-providers.md) and [简体中文](../README.md)",
            "en",
        );
        assert_eq!(en, "See [02](02.md) and [简体中文](../zh/index.md)");
        // Links escaping the manual are untouched.
        let esc = rewrite_links("[docker](../deployment/docker.md)", "zh");
        assert_eq!(esc, "[docker](../deployment/docker.md)");
    }
}
