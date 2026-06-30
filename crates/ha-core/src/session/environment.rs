//! Read-time environment snapshot for the workspace panel.
//!
//! This is UI-only context: it summarizes the session's effective working
//! directory and local git state without changing the model prompt or execution
//! behavior. All filesystem access is anchored through `WorkspaceScope` so HTTP
//! clients cannot ask for arbitrary host paths.

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::filesystem::{git_info, WorkspaceScope, WorktreeInfo};
use crate::session::{effective_working_dir_for_meta, SessionDB, SessionMeta};
use crate::tools::diff_util::{
    compute_line_delta, detect_language, truncate_for_metadata, MAX_METADATA_CONTENT_BYTES,
};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEnvironmentSnapshot {
    pub working_dir: WorkspaceWorkingDirSnapshot,
    pub git: Option<WorkspaceGitSnapshot>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceWorkingDirSnapshot {
    pub path: Option<String>,
    pub source: WorkspaceWorkingDirSource,
    pub exists: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceWorkingDirSource {
    Session,
    Project,
    ProjectDefault,
    None,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitSnapshot {
    pub root: String,
    pub branch: Option<String>,
    pub detached: bool,
    pub head: Option<String>,
    pub worktrees: Vec<WorktreeInfo>,
    pub status: WorkspaceGitStatus,
    pub sync: WorkspaceGitSync,
    pub last_commit: Option<WorkspaceGitCommit>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitStatus {
    pub changed_files: u32,
    pub staged_files: u32,
    pub unstaged_files: u32,
    pub untracked_files: u32,
    pub conflicted_files: u32,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub clean: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitSync {
    pub upstream: Option<String>,
    pub remote: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub state: WorkspaceGitSyncState,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceGitSyncState {
    UpToDate,
    Ahead,
    Behind,
    Diverged,
    NoUpstream,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitCommit {
    pub hash: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitDiff {
    pub kind: &'static str,
    pub changes: Vec<WorkspaceGitFileChange>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitFileChange {
    pub kind: &'static str,
    pub path: String,
    pub action: WorkspaceGitFileAction,
    pub lines_added: u32,
    pub lines_removed: u32,
    pub before: Option<String>,
    pub after: Option<String>,
    pub language: &'static str,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceGitFileAction {
    Create,
    Edit,
    Delete,
}

/// Build the environment snapshot for a session. Missing sessions are treated
/// as bad input because the UI only calls this for an existing session id.
pub fn load_session_environment(
    db: &SessionDB,
    session_id: &str,
) -> Result<WorkspaceEnvironmentSnapshot> {
    let meta = db
        .get_session(session_id)?
        .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
    let working_dir = resolve_working_dir_snapshot(&meta);
    let git = WorkspaceScope::for_session(session_id)
        .ok()
        .and_then(|scope| build_git_snapshot(scope.root()));

    Ok(WorkspaceEnvironmentSnapshot { working_dir, git })
}

/// Build a text diff payload for the current Git working tree relative to
/// HEAD. The shape intentionally mirrors the chat tool `file_changes`
/// metadata so the frontend can render it through the same DiffPanel.
pub fn load_session_git_diff(db: &SessionDB, session_id: &str) -> Result<WorkspaceGitDiff> {
    db.get_session(session_id)?
        .ok_or_else(|| anyhow!("session not found: {session_id}"))?;

    let scope = WorkspaceScope::for_session(session_id)
        .map_err(|e| anyhow!("session has no workspace scope: {e}"))?;
    load_git_diff_for_root(scope.root())
}

/// Build a text diff payload for an already-authorized workspace root.
pub(crate) fn load_git_diff_for_root(root: &Path) -> Result<WorkspaceGitDiff> {
    let scope_root = root
        .canonicalize()
        .map_err(|e| anyhow!("cannot resolve workspace root '{}': {}", root.display(), e))?;
    if !scope_root.is_dir() {
        return Err(anyhow!(
            "workspace root is not a directory: {}",
            scope_root.display()
        ));
    }
    let repo_root = run_git(&scope_root, &["rev-parse", "--show-toplevel"])
        .map(|s| PathBuf::from(s.trim()))
        .and_then(|p| p.canonicalize().ok())
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| anyhow!("workspace is not a git repository"))?;
    let pathspec = pathspec_for_scope(&repo_root, &scope_root)?;

    let mut specs = parse_name_status_z(
        &run_git_bytes(
            &repo_root,
            &[
                "diff",
                "--name-status",
                "-z",
                "HEAD",
                "--",
                pathspec.as_str(),
            ],
        )
        .unwrap_or_default(),
    );
    let mut seen: HashSet<String> = specs.iter().map(|spec| spec.path.clone()).collect();

    for path in parse_nul_paths(
        &run_git_bytes(
            &repo_root,
            &[
                "ls-files",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
                pathspec.as_str(),
            ],
        )
        .unwrap_or_default(),
    ) {
        if seen.insert(path.clone()) {
            specs.push(GitChangeSpec {
                status: GitChangeStatus::Added,
                path,
                old_path: None,
            });
        }
    }

    let mut changes = Vec::new();
    for spec in specs {
        if let Some(change) = build_git_file_change(&repo_root, &scope_root, &spec) {
            changes.push(change);
        }
    }

    Ok(WorkspaceGitDiff {
        kind: "file_changes",
        changes,
    })
}

fn resolve_working_dir_snapshot(meta: &SessionMeta) -> WorkspaceWorkingDirSnapshot {
    let mut source = WorkspaceWorkingDirSource::None;
    if meta
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
    {
        source = WorkspaceWorkingDirSource::Session;
    } else if let Some(pid) = meta.project_id.as_deref() {
        source = WorkspaceWorkingDirSource::ProjectDefault;
        if let Some(project_db) = crate::get_project_db() {
            if let Ok(Some(project)) = project_db.get(pid) {
                if project
                    .working_dir
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .is_some()
                {
                    source = WorkspaceWorkingDirSource::Project;
                }
            }
        }
    }

    let path = effective_working_dir_for_meta(meta);
    let exists = path
        .as_deref()
        .map(|p| Path::new(p).is_dir())
        .unwrap_or(false);
    let name = path.as_deref().and_then(display_name_for_path);

    WorkspaceWorkingDirSnapshot {
        path,
        source: if exists || source != WorkspaceWorkingDirSource::None {
            source
        } else {
            WorkspaceWorkingDirSource::None
        },
        exists,
        name,
    }
}

fn display_name_for_path(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some(path.to_string()))
}

fn build_git_snapshot(root: &Path) -> Option<WorkspaceGitSnapshot> {
    let base = git_info(root)?;
    let repo_root = run_git(root, &["rev-parse", "--show-toplevel"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| root.display().to_string());
    let head = run_git(root, &["rev-parse", "--short", "HEAD"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut status =
        parse_status_porcelain(&run_git(root, &["status", "--porcelain=v1"]).unwrap_or_default());
    let (lines_added, lines_removed) =
        parse_numstat(&run_git(root, &["diff", "--numstat", "HEAD", "--"]).unwrap_or_default());
    status.lines_added = lines_added;
    status.lines_removed = lines_removed;
    status.clean = status.changed_files == 0 && status.conflicted_files == 0;

    let sync = build_git_sync(root, base.branch.as_deref());
    let last_commit = parse_last_commit(
        &run_git(root, &["log", "-1", "--pretty=format:%h%x1f%s"]).unwrap_or_default(),
    );

    Some(WorkspaceGitSnapshot {
        root: repo_root,
        detached: base.branch.is_none(),
        branch: base.branch,
        head,
        worktrees: base.worktrees,
        status,
        sync,
        last_commit,
    })
}

fn build_git_sync(root: &Path, branch: Option<&str>) -> WorkspaceGitSync {
    let upstream = run_git(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());
    let (ahead, behind, counts_known) = upstream
        .as_ref()
        .and_then(|_| {
            run_git(
                root,
                &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
            )
        })
        .and_then(|out| parse_ahead_behind(&out).map(|(a, b)| (a, b, true)))
        .unwrap_or((0, 0, false));

    let remote = branch
        .and_then(|b| run_git(root, &["config", "--get", &format!("branch.{b}.remote")]))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            upstream
                .as_deref()
                .and_then(|u| u.split('/').next().map(str::to_string))
        })
        .and_then(|remote_name| run_git(root, &["remote", "get-url", &remote_name]))
        .map(|s| sanitize_remote_url(s.trim()))
        .filter(|s| !s.is_empty());

    let state = classify_sync_state(upstream.is_some(), counts_known, ahead, behind);
    WorkspaceGitSync {
        upstream,
        remote,
        ahead,
        behind,
        state,
    }
}

fn classify_sync_state(
    has_upstream: bool,
    counts_known: bool,
    ahead: u32,
    behind: u32,
) -> WorkspaceGitSyncState {
    if !has_upstream {
        WorkspaceGitSyncState::NoUpstream
    } else if !counts_known {
        WorkspaceGitSyncState::Unknown
    } else if ahead > 0 && behind > 0 {
        WorkspaceGitSyncState::Diverged
    } else if ahead > 0 {
        WorkspaceGitSyncState::Ahead
    } else if behind > 0 {
        WorkspaceGitSyncState::Behind
    } else {
        WorkspaceGitSyncState::UpToDate
    }
}

fn run_git(root: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(root).args(args);
    crate::platform::hide_console(&mut cmd);
    let output = cmd.output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

fn run_git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.current_dir(root).args(args);
    crate::platform::hide_console(&mut cmd);
    let output = cmd.output()?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(anyhow!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GitChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
struct GitChangeSpec {
    status: GitChangeStatus,
    path: String,
    old_path: Option<String>,
}

fn parse_name_status_z(bytes: &[u8]) -> Vec<GitChangeSpec> {
    let fields: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    let mut specs = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let status_raw = fields[i].as_str();
        i += 1;
        let Some(code) = status_raw.chars().next() else {
            continue;
        };
        let status = match code {
            'A' => GitChangeStatus::Added,
            'M' | 'T' => GitChangeStatus::Modified,
            'D' => GitChangeStatus::Deleted,
            'R' | 'C' => GitChangeStatus::Renamed,
            'U' => GitChangeStatus::Other,
            _ => GitChangeStatus::Other,
        };
        if status == GitChangeStatus::Renamed {
            let Some(old_path) = fields.get(i).cloned() else {
                break;
            };
            let Some(path) = fields.get(i + 1).cloned() else {
                break;
            };
            i += 2;
            specs.push(GitChangeSpec {
                status,
                path,
                old_path: Some(old_path),
            });
            continue;
        }
        let Some(path) = fields.get(i).cloned() else {
            break;
        };
        i += 1;
        specs.push(GitChangeSpec {
            status,
            path,
            old_path: None,
        });
    }
    specs
}

fn parse_nul_paths(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

fn pathspec_for_scope(repo_root: &Path, scope_root: &Path) -> Result<String> {
    let rel = scope_root
        .strip_prefix(repo_root)
        .map_err(|_| anyhow!("session workspace is outside the git repository"))?;
    if rel.as_os_str().is_empty() {
        return Ok(".".to_string());
    }
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

fn git_rel_to_abs_under_scope(repo_root: &Path, scope_root: &Path, rel: &str) -> Option<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return None;
    }
    if rel_path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    let abs = repo_root.join(rel_path);
    if abs.starts_with(scope_root) {
        Some(abs)
    } else {
        None
    }
}

fn build_git_file_change(
    repo_root: &Path,
    scope_root: &Path,
    spec: &GitChangeSpec,
) -> Option<WorkspaceGitFileChange> {
    let abs_path = git_rel_to_abs_under_scope(repo_root, scope_root, &spec.path)?;
    let head_path = spec.old_path.as_deref().unwrap_or(&spec.path);
    let before = match (
        spec.status,
        git_rel_to_abs_under_scope(repo_root, scope_root, head_path),
    ) {
        (GitChangeStatus::Added, _) | (_, None) => None,
        _ => read_head_text_for_diff(repo_root, head_path),
    };
    let after = match spec.status {
        GitChangeStatus::Deleted => None,
        _ => read_worktree_text_for_diff(&abs_path, scope_root),
    };

    if before.is_none() && after.is_none() {
        return None;
    }

    let action = match spec.status {
        GitChangeStatus::Added => WorkspaceGitFileAction::Create,
        GitChangeStatus::Deleted => WorkspaceGitFileAction::Delete,
        _ => WorkspaceGitFileAction::Edit,
    };
    let before_text = before.as_ref().map(|s| s.0.as_str()).unwrap_or("");
    let after_text = after.as_ref().map(|s| s.0.as_str()).unwrap_or("");
    let (mut lines_added, mut lines_removed) = compute_line_delta(before_text, after_text);
    if before.as_ref().is_some_and(|s| s.1) || after.as_ref().is_some_and(|s| s.1) {
        if let Some((added, removed)) = git_numstat_for_path(repo_root, &spec.path) {
            lines_added = added;
            lines_removed = removed;
        }
    }

    let truncated = before.as_ref().is_some_and(|s| s.1) || after.as_ref().is_some_and(|s| s.1);

    Some(WorkspaceGitFileChange {
        kind: "file_change",
        path: abs_path.to_string_lossy().into_owned(),
        action,
        lines_added,
        lines_removed,
        before: before.map(|s| s.0),
        after: after.map(|s| s.0),
        language: detect_language(&spec.path),
        truncated,
    })
}

fn read_head_text_for_diff(repo_root: &Path, rel_path: &str) -> Option<(String, bool)> {
    let object = format!("HEAD:{rel_path}");
    let size = run_git(repo_root, &["cat-file", "-s", &object])?
        .trim()
        .parse::<usize>()
        .ok()?;
    if size > MAX_METADATA_CONTENT_BYTES {
        return Some((String::new(), true));
    }
    let bytes = run_git_bytes(repo_root, &["show", &object]).ok()?;
    let content = String::from_utf8(bytes).ok()?;
    Some(truncate_for_metadata(&content))
}

fn read_worktree_text_for_diff(path: &Path, scope_root: &Path) -> Option<(String, bool)> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        let target = std::fs::read_link(path).ok()?;
        return Some((target.to_string_lossy().into_owned(), false));
    }
    if !file_type.is_file() {
        return None;
    }
    let canonical = path.canonicalize().ok()?;
    if !canonical.starts_with(scope_root) {
        return None;
    }
    if metadata.len() as usize > MAX_METADATA_CONTENT_BYTES {
        return Some((String::new(), true));
    }
    let content = std::fs::read_to_string(canonical).ok()?;
    Some(truncate_for_metadata(&content))
}

fn git_numstat_for_path(repo_root: &Path, rel_path: &str) -> Option<(u32, u32)> {
    let out = run_git(repo_root, &["diff", "--numstat", "HEAD", "--", rel_path])?;
    let mut parts = out.split_whitespace();
    let added = parts.next()?.parse::<u32>().ok()?;
    let removed = parts.next()?.parse::<u32>().ok()?;
    Some((added, removed))
}

fn parse_status_porcelain(out: &str) -> WorkspaceGitStatus {
    let mut status = WorkspaceGitStatus::default();
    for line in out.lines() {
        if line.len() < 2 {
            continue;
        }
        let mut chars = line.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');
        if x == '!' && y == '!' {
            continue;
        }

        status.changed_files += 1;
        if x == '?' && y == '?' {
            status.untracked_files += 1;
            continue;
        }
        if is_conflict_status(x, y) {
            status.conflicted_files += 1;
        }
        if x != ' ' {
            status.staged_files += 1;
        }
        if y != ' ' {
            status.unstaged_files += 1;
        }
    }
    status.clean = status.changed_files == 0 && status.conflicted_files == 0;
    status
}

fn is_conflict_status(x: char, y: char) -> bool {
    x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D')
}

fn parse_numstat(out: &str) -> (u64, u64) {
    let mut added = 0;
    let mut removed = 0;
    for line in out.lines() {
        let mut parts = line.split_whitespace();
        let Some(a) = parts.next() else { continue };
        let Some(r) = parts.next() else { continue };
        if let Ok(n) = a.parse::<u64>() {
            added += n;
        }
        if let Ok(n) = r.parse::<u64>() {
            removed += n;
        }
    }
    (added, removed)
}

fn parse_ahead_behind(out: &str) -> Option<(u32, u32)> {
    let mut parts = out.split_whitespace();
    let ahead = parts.next()?.parse().ok()?;
    let behind = parts.next()?.parse().ok()?;
    Some((ahead, behind))
}

fn parse_last_commit(out: &str) -> Option<WorkspaceGitCommit> {
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (hash, subject) = trimmed.split_once('\u{1f}')?;
    if hash.is_empty() {
        return None;
    }
    Some(WorkspaceGitCommit {
        hash: hash.to_string(),
        subject: subject.to_string(),
    })
}

fn sanitize_remote_url(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string();
    }
    if let Some((_, rest)) = raw.split_once('@') {
        return strip_query_fragment(rest).to_string();
    }
    strip_query_fragment(raw).to_string()
}

fn strip_query_fragment(raw: &str) -> &str {
    raw.find(['?', '#']).map(|idx| &raw[..idx]).unwrap_or(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_status_porcelain_counts_changed_kinds() {
        let status = parse_status_porcelain(concat!(
            " M src/main.rs\n",
            "M  Cargo.toml\n",
            "?? new.txt\n",
            "UU conflicted.txt\n",
            "R  old.txt -> new-name.txt\n",
            "!! ignored.log\n",
        ));
        assert_eq!(status.changed_files, 5);
        assert_eq!(status.staged_files, 3);
        assert_eq!(status.unstaged_files, 2);
        assert_eq!(status.untracked_files, 1);
        assert_eq!(status.conflicted_files, 1);
        assert!(!status.clean);
    }

    #[test]
    fn parses_numstat_and_skips_binary_markers() {
        assert_eq!(
            parse_numstat("10\t2\ta.rs\n-\t-\timage.png\n3\t0\tb.ts\n"),
            (13, 2)
        );
    }

    #[test]
    fn parses_ahead_behind_counts() {
        assert_eq!(parse_ahead_behind("3\t7\n"), Some((3, 7)));
        assert_eq!(parse_ahead_behind("bad\t7\n"), None);
    }

    #[test]
    fn classifies_sync_states() {
        assert_eq!(
            classify_sync_state(false, false, 0, 0),
            WorkspaceGitSyncState::NoUpstream
        );
        assert_eq!(
            classify_sync_state(true, false, 0, 0),
            WorkspaceGitSyncState::Unknown
        );
        assert_eq!(
            classify_sync_state(true, true, 2, 0),
            WorkspaceGitSyncState::Ahead
        );
        assert_eq!(
            classify_sync_state(true, true, 0, 4),
            WorkspaceGitSyncState::Behind
        );
        assert_eq!(
            classify_sync_state(true, true, 2, 4),
            WorkspaceGitSyncState::Diverged
        );
        assert_eq!(
            classify_sync_state(true, true, 0, 0),
            WorkspaceGitSyncState::UpToDate
        );
    }

    #[test]
    fn sanitizes_remote_urls() {
        assert_eq!(
            sanitize_remote_url("https://token@example.com/org/repo.git"),
            "https://example.com/org/repo.git"
        );
        assert_eq!(
            sanitize_remote_url("https://user:secret@example.com/org/repo.git"),
            "https://example.com/org/repo.git"
        );
        assert_eq!(
            sanitize_remote_url("https://token@example.com/org/repo.git?access_token=secret#frag"),
            "https://example.com/org/repo.git"
        );
        assert_eq!(
            sanitize_remote_url("git@example.com:org/repo.git"),
            "example.com:org/repo.git"
        );
        assert_eq!(
            sanitize_remote_url("git@example.com:org/repo.git?token=secret"),
            "example.com:org/repo.git"
        );
    }

    #[test]
    fn parses_last_commit_summary() {
        assert_eq!(
            parse_last_commit("abc123\u{1f}Initial commit"),
            Some(WorkspaceGitCommit {
                hash: "abc123".to_string(),
                subject: "Initial commit".to_string(),
            })
        );
    }

    #[test]
    fn display_name_handles_root_paths() {
        let name = display_name_for_path("/");
        assert_eq!(name.as_deref(), Some("/"));
        let nested = PathBuf::from("/tmp/hope-agent");
        assert_eq!(
            display_name_for_path(&nested.to_string_lossy()).as_deref(),
            Some("hope-agent")
        );
    }
}
