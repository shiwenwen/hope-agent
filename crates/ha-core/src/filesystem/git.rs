//! Read-only git context (current branch + worktree list) for the file
//! browser. Pure `std::process::Command` shelling out to `git`, mirroring the
//! approach in `plan/git.rs`; no git library dependency.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// Ensure a Git subprocess discovers its repository from `current_dir` rather
/// than inheriting repository-local state from a parent Git process or hook.
pub(crate) fn isolate_repository_env(cmd: &mut Command) {
    const LOCAL_REPOSITORY_ENV: &[&str] = &[
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_OBJECT_DIRECTORY",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_NO_REPLACE_OBJECTS",
        "GIT_REPLACE_REF_BASE",
        "GIT_PREFIX",
        "GIT_SHALLOW_FILE",
        "GIT_COMMON_DIR",
    ];

    for name in LOCAL_REPOSITORY_ENV {
        cmd.env_remove(name);
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    /// Current branch name, or `None` when detached / unreadable.
    pub branch: Option<String>,
    pub branches: Vec<GitBranchInfo>,
    pub dirty: GitDirtySummary,
    pub worktrees: Vec<WorktreeInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchInfo {
    pub name: String,
    pub full_ref: String,
    pub kind: GitBranchKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    pub is_current: bool,
    pub is_checked_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_out_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitBranchKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitDirtySummary {
    pub staged_files: u32,
    pub unstaged_files: u32,
    pub untracked_files: u32,
    pub conflicted_files: u32,
    pub changed_files: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    /// Absolute worktree path.
    pub path: String,
    /// Branch the worktree has checked out, or `None` when detached.
    pub branch: Option<String>,
    /// `true` for the worktree that `root` itself resolves to.
    pub is_current: bool,
}

/// Read git branch + worktree list for `root`. Returns `None` when `root` is
/// not inside a git work tree or `git` is unavailable — callers then simply
/// omit the git UI.
pub fn git_info(root: &Path) -> Option<GitInfo> {
    if !is_inside_work_tree(root) {
        return None;
    }

    let branch = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD");

    let canon_root = root.canonicalize().ok();
    let worktrees = run_git(root, &["worktree", "list", "--porcelain"])
        .map(|out| parse_worktrees(&out, canon_root.as_deref()))
        .unwrap_or_default();
    let current_ref = run_git(root, &["symbolic-ref", "--quiet", "HEAD"])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let branches = run_git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)\t%(refname:short)\t%(symref)",
            "refs/heads",
            "refs/remotes",
        ],
    )
    .map(|out| parse_branches(&out, current_ref.as_deref(), &worktrees))
    .unwrap_or_default();
    let dirty = run_git(root, &["status", "--porcelain=v1"])
        .map(|out| parse_dirty_summary(&out))
        .unwrap_or_default();

    Some(GitInfo {
        branch,
        branches,
        dirty,
        worktrees,
    })
}

/// Cheap probe: is `dir` inside a git work tree?
pub fn is_inside_work_tree(dir: &Path) -> bool {
    let mut cmd = Command::new("git");
    isolate_repository_env(&mut cmd);
    cmd.current_dir(dir)
        .args(["rev-parse", "--is-inside-work-tree"]);
    crate::platform::hide_console(&mut cmd);
    cmd.output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// Security gate for the read-only `path` browse scope: is `target_canon` (an
/// already-canonicalized directory) one of the worktrees of the git repository
/// at `base`? This anchors worktree-jump browsing to the *current* session /
/// project repository, so a client can never jump to an arbitrary git repo on
/// the host — only between the base repo's own worktrees. Runs a single
/// `git worktree list` (no branch probe needed here).
pub fn is_worktree_of(base: &Path, target_canon: &Path) -> bool {
    run_git(base, &["worktree", "list", "--porcelain"])
        .map(|out| {
            parse_worktrees(&out, None).iter().any(|w| {
                Path::new(&w.path)
                    .canonicalize()
                    .map(|c| c == *target_canon)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn run_git(root: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    isolate_repository_env(&mut cmd);
    cmd.current_dir(root).args(args);
    crate::platform::hide_console(&mut cmd);
    let output = cmd.output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

/// Parse `git worktree list --porcelain`. Each block starts with a
/// `worktree <abs-path>` line, optionally followed by `branch refs/heads/<b>`
/// (absent when detached). Blocks are separated by blank lines.
fn parse_worktrees(porcelain: &str, canon_root: Option<&Path>) -> Vec<WorktreeInfo> {
    fn flush(
        out: &mut Vec<WorktreeInfo>,
        path: &mut Option<String>,
        branch: &mut Option<String>,
        canon_root: Option<&Path>,
    ) {
        let b = branch.take();
        if let Some(p) = path.take() {
            let is_current = canon_root
                .and_then(|cr| Path::new(&p).canonicalize().ok().map(|pc| pc == cr))
                .unwrap_or(false);
            out.push(WorktreeInfo {
                path: p,
                branch: b,
                is_current,
            });
        }
    }

    let mut out = Vec::new();
    let mut cur_path: Option<String> = None;
    let mut cur_branch: Option<String> = None;
    for line in porcelain.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(&mut out, &mut cur_path, &mut cur_branch, canon_root);
            cur_path = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            cur_branch = Some(rest.trim_start_matches("refs/heads/").to_string());
        }
    }
    flush(&mut out, &mut cur_path, &mut cur_branch, canon_root);
    out
}

fn parse_branches(
    output: &str,
    current_ref: Option<&str>,
    worktrees: &[WorktreeInfo],
) -> Vec<GitBranchInfo> {
    let checked_out = worktrees
        .iter()
        .filter_map(|worktree| {
            worktree
                .branch
                .as_deref()
                .map(|branch| (branch, worktree.path.as_str()))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let mut branches = output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let full_ref = fields.next()?.trim();
            let name = fields.next()?.trim();
            let symref = fields.next().unwrap_or_default().trim();
            if full_ref.is_empty()
                || name.is_empty()
                || !symref.is_empty()
                || (full_ref.starts_with("refs/remotes/") && name.ends_with("/HEAD"))
            {
                return None;
            }
            let (kind, remote) = if let Some(remote_ref) = full_ref.strip_prefix("refs/remotes/") {
                let remote = remote_ref
                    .split_once('/')
                    .map(|(remote, _)| remote.to_string());
                (GitBranchKind::Remote, remote)
            } else if full_ref.starts_with("refs/heads/") {
                (GitBranchKind::Local, None)
            } else {
                return None;
            };
            Some(GitBranchInfo {
                name: name.to_string(),
                full_ref: full_ref.to_string(),
                kind,
                remote,
                is_current: current_ref == Some(full_ref),
                is_checked_out: kind == GitBranchKind::Local && checked_out.contains_key(name),
                checked_out_path: (kind == GitBranchKind::Local)
                    .then(|| checked_out.get(name).copied())
                    .flatten()
                    .map(str::to_string),
            })
        })
        .collect::<Vec<_>>();
    branches.sort_by(|a, b| {
        (!a.is_current)
            .cmp(&(!b.is_current))
            .then_with(|| (a.kind == GitBranchKind::Remote).cmp(&(b.kind == GitBranchKind::Remote)))
            .then_with(|| a.name.cmp(&b.name))
    });
    branches
}

fn parse_dirty_summary(output: &str) -> GitDirtySummary {
    let mut summary = GitDirtySummary::default();
    for line in output.lines() {
        let bytes = line.as_bytes();
        if bytes.len() < 2 {
            continue;
        }
        summary.changed_files += 1;
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        if x == '?' && y == '?' {
            summary.untracked_files += 1;
            continue;
        }
        if matches!(
            (x, y),
            ('D', 'D')
                | ('A', 'U')
                | ('U', 'D')
                | ('U', 'A')
                | ('D', 'U')
                | ('A', 'A')
                | ('U', 'U')
        ) {
            summary.conflicted_files += 1;
            continue;
        }
        if x != ' ' {
            summary.staged_files += 1;
        }
        if y != ' ' {
            summary.unstaged_files += 1;
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_with_branch_and_detached() {
        let porcelain = "\
worktree /repo/main
HEAD abc123
branch refs/heads/main

worktree /repo/feature
HEAD def456
branch refs/heads/feature

worktree /repo/detached
HEAD 999000
detached
";
        let wts = parse_worktrees(porcelain, None);
        assert_eq!(wts.len(), 3);
        assert_eq!(wts[0].path, "/repo/main");
        assert_eq!(wts[0].branch.as_deref(), Some("main"));
        assert_eq!(wts[1].branch.as_deref(), Some("feature"));
        assert_eq!(wts[2].path, "/repo/detached");
        assert_eq!(wts[2].branch, None);
    }

    #[test]
    fn empty_porcelain_yields_no_worktrees() {
        assert!(parse_worktrees("", None).is_empty());
    }

    #[test]
    fn parses_local_and_remote_branches() {
        let worktrees = vec![WorktreeInfo {
            path: "/repo/main".into(),
            branch: Some("main".into()),
            is_current: true,
        }];
        let branches = parse_branches(
            "refs/heads/main\tmain\t\nrefs/heads/feature\tfeature\t\nrefs/remotes/origin/feature\torigin/feature\t\nrefs/remotes/origin/HEAD\torigin/HEAD\trefs/remotes/origin/main\n",
            Some("refs/heads/main"),
            &worktrees,
        );
        assert_eq!(branches.len(), 3);
        assert!(branches[0].is_current);
        assert!(branches[0].is_checked_out);
        assert_eq!(branches[2].kind, GitBranchKind::Remote);
        assert_eq!(branches[2].remote.as_deref(), Some("origin"));
    }

    #[test]
    fn summarizes_dirty_status() {
        let summary = parse_dirty_summary(
            "M  staged.rs\n M unstaged.rs\nMM both.rs\n?? new.rs\nUU conflict.rs\n",
        );
        assert_eq!(summary.staged_files, 2);
        assert_eq!(summary.unstaged_files, 2);
        assert_eq!(summary.untracked_files, 1);
        assert_eq!(summary.conflicted_files, 1);
        assert_eq!(summary.changed_files, 5);
    }
}
