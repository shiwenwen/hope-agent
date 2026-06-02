//! Project/session-scoped filesystem access for the file browser.
//!
//! Every read and write the file-browser API performs goes through a
//! [`WorkspaceScope`], which pins operations to a single working-directory root
//! and rejects any path that escapes it (canonicalize + `starts_with`, failing
//! closed). This is the one chokepoint — handlers never join paths themselves,
//! so a `../`, an absolute path, or a symlink that escapes the root can never
//! reach `std::fs`.

use std::path::{Component, Path, PathBuf};

use super::{FilesystemError, Result};

/// Separator for the `path` scope's encoded id
/// (`base_scope ∣ base_scope_id ∣ target_abs`). U+001F (Unit Separator) can
/// never appear in a scope kind, a session/project id, or a filesystem path,
/// so the split is unambiguous. The frontend builds the same triple.
const PATH_SCOPE_SEP: char = '\u{1f}';

/// A working-directory root that all file-browser operations are confined to.
/// Constructed from a session (its effective working dir) or a project (its
/// explicit `working_dir` or default workspace).
pub struct WorkspaceScope {
    /// Canonical, symlink-free absolute root. All resolved paths must live
    /// under this prefix.
    root: PathBuf,
}

impl WorkspaceScope {
    /// Dispatch by scope kind: `"session"` → [`Self::for_session`],
    /// `"project"` → [`Self::for_project`]. The single entry point the command
    /// layers use so the kind string is validated in exactly one place.
    pub fn resolve(kind: &str, id: &str) -> Result<Self> {
        match kind {
            "session" => Self::for_session(id),
            "project" => Self::for_project(id),
            "path" => Self::for_path(id),
            other => Err(FilesystemError::bad_input(format!(
                "invalid scope: {other}"
            ))),
        }
    }

    /// Like [`Self::resolve`] but rejects read-only scopes. The `"path"` scope
    /// (git-worktree jump browsing) is read-only — write/delete/rename/mkdir/
    /// upload must route through here so a worktree view can't mutate files.
    pub fn resolve_writable(kind: &str, id: &str) -> Result<Self> {
        if kind == "path" {
            return Err(FilesystemError::bad_input("this view is read-only"));
        }
        Self::resolve(kind, id)
    }

    /// Read-only worktree-jump browse scope. The `id` is an opaque triple
    /// `"{base_scope}\x1f{base_scope_id}\x1f{target_abs}"` (U+001F separator):
    /// the target is accepted **only if git reports it as one of the worktrees
    /// of the base (session/project) repository**. This anchors the jump to the
    /// current repo — a client can never point `path` at an arbitrary git repo
    /// on the host (which the old "inside any git work tree" gate allowed,
    /// escaping the per-session/project boundary over the HTTP read endpoints).
    pub fn for_path(id: &str) -> Result<Self> {
        let mut parts = id.splitn(3, PATH_SCOPE_SEP);
        let base_scope = parts.next().unwrap_or("");
        let (base_scope_id, target) = match (parts.next(), parts.next()) {
            (Some(b), Some(t)) if !t.trim().is_empty() => (b, t),
            _ => return Err(FilesystemError::bad_input("invalid path scope")),
        };

        // The base must resolve through a real session/project scope (never
        // another `path`, so there is no recursion and no way to launder an
        // arbitrary directory in as the anchor).
        let base = match base_scope {
            "session" => Self::for_session(base_scope_id)?,
            "project" => Self::for_project(base_scope_id)?,
            _ => {
                return Err(FilesystemError::bad_input(
                    "invalid base scope for path jump",
                ))
            }
        };

        let target_root = Path::new(target.trim()).canonicalize().map_err(|e| {
            FilesystemError::bad_input(format!("cannot resolve path '{}': {}", target, e))
        })?;
        if !target_root.is_dir() {
            return Err(FilesystemError::bad_input("path is not a directory"));
        }
        if !super::git::is_worktree_of(base.root(), &target_root) {
            return Err(FilesystemError::bad_input(
                "path is not a worktree of the current repository",
            ));
        }
        Ok(Self { root: target_root })
    }

    /// Scope to a session's effective working directory (session-level dir →
    /// project dir → default workspace). Errors if the session has no working
    /// directory (non-project session that never selected one).
    pub fn for_session(session_id: &str) -> Result<Self> {
        let dir = crate::session::effective_session_working_dir(Some(session_id))
            .ok_or_else(|| FilesystemError::bad_input("session has no working directory"))?;
        Self::from_root(&dir)
    }

    /// Scope to a project's working directory (explicit `working_dir`, else the
    /// lazily-created default workspace).
    pub fn for_project(project_id: &str) -> Result<Self> {
        let db = crate::get_project_db()
            .ok_or_else(|| FilesystemError::internal("project db not initialized"))?;
        let dir = crate::project::resolve_project_dir(project_id, &db)
            .map_err(|e| FilesystemError::bad_input(e.to_string()))?;
        Self::from_root(&dir.to_string_lossy())
    }

    fn from_root(dir: &str) -> Result<Self> {
        let root = Path::new(dir).canonicalize().map_err(|e| {
            FilesystemError::internal(format!("cannot resolve workspace root '{}': {}", dir, e))
        })?;
        if !root.is_dir() {
            return Err(FilesystemError::bad_input(format!(
                "workspace root is not a directory: {}",
                root.display()
            )));
        }
        Ok(Self { root })
    }

    /// The canonical workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether an already-canonical absolute path lives under this root. Used by
    /// authorization checks outside the rel-path API (e.g. the HTTP preview/
    /// download gate broadening "tool-referenced" to "anything in the working
    /// directory"). The caller must canonicalize before calling.
    pub fn contains(&self, canonical_abs: &Path) -> bool {
        canonical_abs.starts_with(&self.root)
    }

    /// Turn an absolute path under the root into the `/`-separated relative path
    /// the API speaks. Returns `""` for the root itself.
    pub fn rel_of(&self, abs: &Path) -> String {
        abs.strip_prefix(&self.root)
            .ok()
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    }

    /// Resolve a relative path that must already exist, verifying containment.
    /// Used for read / delete / rename source.
    pub fn resolve_existing(&self, rel: &str) -> Result<PathBuf> {
        let joined = self.join_checked(rel)?;
        let canon = joined
            .canonicalize()
            .map_err(|_| FilesystemError::bad_input("path not found"))?;
        self.ensure_contained(&canon)?;
        Ok(canon)
    }

    /// Resolve a relative path that may not exist yet (write / mkdir / rename
    /// destination), verifying containment via the nearest existing ancestor so
    /// a symlinked ancestor cannot smuggle the target outside the root.
    pub fn resolve_new(&self, rel: &str) -> Result<PathBuf> {
        let joined = self.join_checked(rel)?;

        let mut ancestor = joined.as_path();
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        let canon_ancestor = loop {
            match ancestor.canonicalize() {
                Ok(c) => break c,
                Err(_) => {
                    let file = ancestor
                        .file_name()
                        .ok_or_else(|| FilesystemError::bad_input("invalid path"))?;
                    tail.push(file.to_os_string());
                    ancestor = ancestor
                        .parent()
                        .ok_or_else(|| FilesystemError::bad_input("invalid path"))?;
                }
            }
        };

        // When the target itself doesn't exist (tail non-empty), its nearest
        // existing ancestor must be a directory — otherwise a path component is
        // a regular file (e.g. `notes.txt/sub`) and the operation would fail
        // deep in std::fs with an opaque "Not a directory" error.
        if !tail.is_empty() && !canon_ancestor.is_dir() {
            return Err(FilesystemError::bad_input(
                "a path component is not a directory",
            ));
        }
        let mut full = canon_ancestor;
        for part in tail.iter().rev() {
            full.push(part);
        }
        self.ensure_contained(&full)?;
        Ok(full)
    }

    /// Pre-join validation: reject absolute paths and `..` traversal before the
    /// path ever touches the filesystem.
    fn join_checked(&self, rel: &str) -> Result<PathBuf> {
        let rel = rel.trim().trim_start_matches('/');
        if rel.contains('\0') {
            return Err(FilesystemError::bad_input("invalid path"));
        }
        let p = Path::new(rel);
        for comp in p.components() {
            match comp {
                Component::ParentDir => {
                    return Err(FilesystemError::bad_input("path escapes workspace"))
                }
                Component::Prefix(_) | Component::RootDir => {
                    return Err(FilesystemError::bad_input("path must be relative"))
                }
                _ => {}
            }
        }
        Ok(self.root.join(p))
    }

    fn ensure_contained(&self, canon: &Path) -> Result<()> {
        if canon.starts_with(&self.root) {
            Ok(())
        } else {
            // Uniform message — never reveal whether the target merely doesn't
            // exist vs. lives outside the root, so this can't be a probe.
            Err(FilesystemError::bad_input("path outside workspace"))
        }
    }
}
