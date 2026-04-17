//! Conditional skill activation (`paths:` frontmatter support).
//!
//! Skills declaring `paths: [...]` are hidden from the catalog until a file
//! matching one of those gitignore-style patterns is touched in the current
//! session (via `read`/`write`/`edit`/`apply_patch`). Once activated they
//! remain in the catalog for the rest of the session, surviving compaction.
//!
//! State lives in a process-wide in-memory cache keyed by `session_id`,
//! backed by the `session_skill_activation` SQLite table so App restart
//! preserves the activation set.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::types::SkillEntry;

static ACTIVATED_CONDITIONAL: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    ACTIVATED_CONDITIONAL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Return the set of conditional skills already activated for this session.
/// Hydrates from the DB on first access within the process.
pub fn activated_skill_names(session_id: &str) -> HashSet<String> {
    if session_id.is_empty() {
        return HashSet::new();
    }
    {
        let map = cache().lock().expect("activation cache poisoned");
        if let Some(set) = map.get(session_id) {
            return set.clone();
        }
    }

    // Cache miss — hydrate from DB outside the lock so we don't block other
    // sessions' activation lookups on SQLite IO.
    let hydrated = match crate::globals::get_session_db() {
        Some(db) => db.load_skill_activations(session_id).unwrap_or_default(),
        None => Vec::new(),
    };
    let set: HashSet<String> = hydrated.into_iter().collect();
    // A concurrent writer may have inserted an entry while we were hydrating —
    // prefer theirs (it's a superset of whatever we just loaded).
    let mut map = cache().lock().expect("activation cache poisoned");
    map.entry(session_id.to_string())
        .or_insert_with(|| set.clone());
    map.get(session_id).cloned().unwrap_or(set)
}

/// Match a set of touched paths against all skills in the catalog that declare
/// `paths:`, persist newly-activated ones, and return the names that were
/// activated in this call. Returns an empty Vec when nothing new lights up.
pub fn activate_skills_for_paths(
    session_id: &str,
    touched: &[String],
    cwd: &str,
    skills: &[SkillEntry],
) -> Vec<String> {
    if session_id.is_empty() || touched.is_empty() {
        return Vec::new();
    }

    let already = activated_skill_names(session_id);
    let cwd_path = if cwd.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(cwd)
    };

    let mut newly: Vec<String> = Vec::new();

    for skill in skills {
        let Some(ref patterns) = skill.paths else { continue };
        if patterns.is_empty() {
            continue;
        }
        if already.contains(&skill.name) {
            continue;
        }

        let matcher = match build_matcher(&cwd_path, patterns) {
            Some(m) => m,
            None => continue,
        };

        if touched
            .iter()
            .any(|p| match_path(&matcher, &cwd_path, p))
        {
            newly.push(skill.name.clone());
        }
    }

    if newly.is_empty() {
        return Vec::new();
    }

    // Persist and update cache; DB is source of truth, cache is hot copy.
    let persisted = match crate::globals::get_session_db() {
        Some(db) => db.insert_skill_activations(session_id, &newly).unwrap_or_default(),
        None => newly.clone(),
    };

    {
        let mut map = cache().lock().expect("activation cache poisoned");
        let entry = map.entry(session_id.to_string()).or_default();
        for name in &persisted {
            entry.insert(name.clone());
        }
    }

    persisted
}

/// Clear cached activations for a session (call when the session is deleted).
pub fn clear_session_activation(session_id: &str) {
    let mut map = cache().lock().expect("activation cache poisoned");
    map.remove(session_id);
}

/// Reset the entire cache — used after skill directory mutations so that
/// removed skills don't linger in stale entries. DB rows are kept (cheap);
/// next read hydrates again.
pub fn reset_activation_cache() {
    let mut map = cache().lock().expect("activation cache poisoned");
    map.clear();
}

fn build_matcher(base: &Path, patterns: &[String]) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(base);
    for pattern in patterns {
        if builder.add_line(None, pattern).is_err() {
            // Skip malformed patterns but keep building — one bad rule should
            // not poison the whole skill.
            continue;
        }
    }
    builder.build().ok()
}

fn match_path(matcher: &Gitignore, base: &Path, path: &str) -> bool {
    let candidate = Path::new(path);
    let abs_candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(candidate)
    };
    // `matched_path_or_any_parents` requires the path to live under the root;
    // fall back to `matched` (with a relativized path) when we can make the
    // input look relative. For paths outside cwd with no relative form, treat
    // as a non-match.
    let relative = abs_candidate
        .strip_prefix(base)
        .ok()
        .map(|p| p.to_path_buf());
    let m = match relative {
        Some(rel) => matcher.matched(&rel, false),
        None => matcher.matched(&abs_candidate, false),
    };
    m.is_ignore()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, patterns: &[&str]) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            aliases: Vec::new(),
            description: String::new(),
            when_to_use: None,
            source: "managed".into(),
            file_path: format!("/tmp/{name}/SKILL.md"),
            base_dir: format!("/tmp/{name}"),
            requires: Default::default(),
            skill_key: None,
            user_invocable: None,
            disable_model_invocation: None,
            command_dispatch: None,
            command_tool: None,
            command_arg_mode: None,
            command_arg_placeholder: None,
            command_arg_options: None,
            command_prompt_template: None,
            install: vec![],
            allowed_tools: vec![],
            context_mode: None,
            agent: None,
            effort: None,
            paths: if patterns.is_empty() {
                None
            } else {
                Some(patterns.iter().map(|s| s.to_string()).collect())
            },
            status: crate::skills::types::SkillStatus::Active,
            authored_by: None,
            rationale: None,
        }
    }

    #[test]
    fn matcher_accepts_relative_files_under_cwd() {
        let base = Path::new("/tmp/project");
        let m = build_matcher(base, &["*.py".into()]).unwrap();
        assert!(match_path(&m, base, "src/foo.py"));
        assert!(!match_path(&m, base, "src/foo.rs"));
    }

    #[test]
    fn matcher_accepts_absolute_paths() {
        let base = Path::new("/tmp/project");
        let m = build_matcher(base, &["docs/**".into()]).unwrap();
        assert!(match_path(&m, base, "/tmp/project/docs/README.md"));
        assert!(!match_path(&m, base, "/tmp/other/docs/README.md"));
    }

    #[test]
    fn skill_without_paths_is_never_activated() {
        // A skill with paths: None is "always visible" — the activator should
        // leave it alone, so nothing gets persisted/returned.
        let skills = vec![skill("global", &[])];
        let activated = activate_skills_for_paths(
            "test-session-no-paths",
            &["/tmp/anything.py".into()],
            "/tmp",
            &skills,
        );
        assert!(activated.is_empty());
    }
}
