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
            other => Err(FilesystemError::bad_input(format!(
                "invalid scope: {other}"
            ))),
        }
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
