use std::collections::HashMap;

use super::requirements::check_requirements_detail;
use super::types::*;

// ── Slash Command Integration ───────────────────────────────────

/// Normalize a skill name into a valid slash command name.
/// - Lowercase, non-alphanumeric -> `_`, truncate to 32 chars, deduplicate underscores.
pub fn normalize_skill_command_name(name: &str) -> String {
    let normalized: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    // Deduplicate underscores and trim edges
    let mut result = String::new();
    let mut prev_underscore = true; // Treat start as underscore to trim leading
    for c in normalized.chars() {
        if c == '_' {
            if !prev_underscore {
                result.push(c);
            }
            prev_underscore = true;
        } else {
            result.push(c);
            prev_underscore = false;
        }
    }
    // Trim trailing underscore
    while result.ends_with('_') {
        result.pop();
    }
    // Truncate to 32 chars (safe for ASCII)
    if result.len() > 32 {
        result.truncate(32);
    }
    if result.is_empty() {
        "skill".to_string()
    } else {
        result
    }
}

// ── Health Check ─────────────────────────────────────────────────

/// Check the health status of all skills.
pub fn check_all_skills_status(
    skills: &[SkillEntry],
    disabled: &[String],
    env_check: bool,
    skill_env: &HashMap<String, HashMap<String, String>>,
    allow_bundled: &[String],
) -> Vec<SkillStatusEntry> {
    skills
        .iter()
        .map(|s| {
            let is_disabled = disabled.contains(&s.name);
            let blocked_by_allowlist = if !allow_bundled.is_empty() && s.source == "bundled" {
                let key = s.skill_key.as_deref().unwrap_or(&s.name);
                !allow_bundled.iter().any(|a| a == key || a == &s.name)
            } else {
                false
            };

            let detail = if env_check {
                check_requirements_detail(&s.requires, skill_env.get(&s.name))
            } else {
                RequirementsDetail {
                    eligible: true,
                    ..Default::default()
                }
            };

            let eligible = !is_disabled && !blocked_by_allowlist && detail.eligible;

            SkillStatusEntry {
                name: s.name.clone(),
                source: s.source.clone(),
                eligible,
                disabled: is_disabled,
                blocked_by_allowlist,
                missing_bins: detail.missing_bins,
                missing_any_bins: detail.missing_any_bins,
                missing_env: detail.missing_env,
                missing_config: detail.missing_config,
                has_install: !s.install.is_empty(),
                always: s.requires.always,
            }
        })
        .collect()
}
