//! Edit-commands list — `exec` command patterns that need approval in
//! Default mode but ARE AllowAlways'd-able (unlike dangerous commands).
//!
//! Compared to [`super::dangerous_commands`]:
//! - Dangerous commands: irreversible destruction. Cannot AllowAlways.
//!   YOLO bypasses with `app_warn!` audit.
//! - Edit commands: filesystem-modifying but recoverable. Default mode requires
//!   approval; user can AllowAlways. Smart / YOLO modes ignore this list.
//!
//! Storage: `~/.hope-agent/permission/edit-commands.json`.

/// Default edit patterns shipped with Hope Agent. Users can add / remove
/// via the GUI; "Restore defaults" rewrites the on-disk file with this list.
///
/// Patterns are substring matches (case-sensitive). The trailing space in
/// `"rm "` etc. avoids false positives like `rmagick` or `rmdir-friendly`.
pub const DEFAULT_EDIT_COMMAND_PATTERNS: &[&str] = &[
    // File operations
    "rm ",
    "rm\t",
    "rmdir ",
    "mv ",
    "cp ",
    "mkdir ",
    "touch ",
    "ln ",
    "ln -s",
    // In-place text editing
    "sed -i",
    "awk -i",
    "perl -i",
    // Editors
    "vim ",
    "vi ",
    "nano ",
    "emacs ",
    "code ",
    // Archives (filesystem write)
    "tar -xf",
    "tar -xzf",
    "tar -xvzf",
    "unzip ",
    "gunzip ",
    // Permission changes
    "chmod ",
    "chown ",
    "chgrp ",
    "truncate ",
    // Package managers (filesystem write)
    "npm install",
    "npm i ",
    "pnpm install",
    "pnpm i ",
    "yarn install",
    "yarn add",
    "pip install",
    "pip3 install",
    "uv pip install",
    "brew install",
    "brew upgrade",
    "apt install",
    "apt-get install",
    "cargo install",
    "cargo build",
    "cargo update",
    "go install",
    "go build",
    // Git write operations
    "git commit",
    "git add",
    "git checkout -b",
    "git checkout --",
    "git branch -d",
    "git branch -D",
    "git merge",
    "git rebase",
    "git stash pop",
    "git stash apply",
    "git pull",
    "git fetch ",
    "git tag ",
    "git rm ",
    "git mv ",
    // Build / Make
    "make ",
    "cmake --build",
    "ninja",
    "npm run build",
    "pnpm build",
    "yarn build",
    "npm run dev",
    "pnpm dev",
    // File redirection (coarse — UI hint user to refine)
    "> ",
    ">> ",
    "tee ",
];

/// Currently-active edit-command pattern list. The GUI editor will swap this
/// for a `Lazy<RwLock<Vec<String>>>` once user customization lands.
pub fn current_patterns() -> &'static [&'static str] {
    DEFAULT_EDIT_COMMAND_PATTERNS
}

/// Same case-insensitive substring strategy as
/// [`super::dangerous_commands::matches`]. Both share the same allocation-free
/// helper in [`super::pattern_match`].
pub fn matches(command: &str, patterns: &[&'static str]) -> Option<&'static str> {
    super::pattern_match::first_substring_match_ignore_ascii_case(command, patterns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_non_empty() {
        assert!(!DEFAULT_EDIT_COMMAND_PATTERNS.is_empty());
    }

    #[test]
    fn defaults_include_rm() {
        assert!(DEFAULT_EDIT_COMMAND_PATTERNS.contains(&"rm "));
    }

    #[test]
    fn rm_pattern_has_trailing_separator() {
        // Sanity: ensure we kept the trailing space/tab variants to avoid
        // false-positive matches against unrelated tokens like "rmagick".
        assert!(DEFAULT_EDIT_COMMAND_PATTERNS.contains(&"rm "));
        assert!(DEFAULT_EDIT_COMMAND_PATTERNS.contains(&"rm\t"));
    }
}
