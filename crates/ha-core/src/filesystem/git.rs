//! Read-only git context (current branch + worktree list) for the file
//! browser. Pure `std::process::Command` shelling out to `git`, mirroring the
//! approach in `plan/git.rs`; no git library dependency.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    /// Current branch name, or `None` when detached / unreadable.
    pub branch: Option<String>,
    pub worktrees: Vec<WorktreeInfo>,
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

    Some(GitInfo { branch, worktrees })
}

/// Cheap probe: is `dir` inside a git work tree?
pub fn is_inside_work_tree(dir: &Path) -> bool {
    let mut cmd = Command::new("git");
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
}
