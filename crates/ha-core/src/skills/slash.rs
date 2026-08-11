use std::collections::HashMap;

use super::requirements::check_requirements_detail;
use super::types::*;

// ── Slash Command Integration ───────────────────────────────────

/// Frozen dispatch decision for one explicit Skill slash invocation.
///
/// Both the control-plane slash handler and the typed chat-engine path resolve
/// this from the same trusted [`SkillEntry`]. The frontend may carry the raw
/// slash text and binding, but never supplies the model prompt used here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillSlashDispatch {
    Fork,
    Tool,
    ModelTemplate { message: String },
    ModelInline,
}

/// Resolve the Skill's slash dispatch and, for prompt/template modes, expand
/// the trusted catalog template with the canonical arguments.
///
/// `context: fork` keeps its historical precedence over `command-dispatch`.
/// `command-dispatch: prompt` always selects template semantics; discovery
/// normally fills a missing explicit template from the SKILL.md body.
pub fn resolve_skill_slash_dispatch(skill: &SkillEntry, args: &str) -> SkillSlashDispatch {
    if skill.context_mode.as_deref() == Some("fork") {
        return SkillSlashDispatch::Fork;
    }
    if skill.command_dispatch.as_deref() == Some("tool") {
        return SkillSlashDispatch::Tool;
    }
    if skill.command_dispatch.as_deref() == Some("prompt") {
        return SkillSlashDispatch::ModelTemplate {
            message: expand_skill_prompt_template(
                skill.command_prompt_template.as_deref().unwrap_or(""),
                args,
            ),
        };
    }
    if let Some(template) = skill.command_prompt_template.as_deref() {
        return SkillSlashDispatch::ModelTemplate {
            message: expand_skill_prompt_template(template, args),
        };
    }
    SkillSlashDispatch::ModelInline
}

/// Expand `$ARGUMENTS` in a Skill-owned prompt template. If the template has
/// no placeholder, preserve the existing slash contract by appending a
/// separate user-input section.
pub fn expand_skill_prompt_template(template: &str, args: &str) -> String {
    let normalized = args.trim();
    if template.contains("$ARGUMENTS") {
        template.replace("$ARGUMENTS", normalized)
    } else if !normalized.is_empty() {
        format!("{}\n\nUser input:\n{}", template.trim(), normalized)
    } else {
        template.trim().to_string()
    }
}

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
                hard_blocked: detail.hard_blocked,
                needs_setup: detail.needs_setup,
                disabled: is_disabled,
                blocked_by_allowlist,
                current_os: detail.current_os,
                supported_os: detail.supported_os,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> SkillEntry {
        serde_json::from_value(serde_json::json!({
            "name": "review",
            "description": "test",
            "source": "test",
            "file_path": "/tmp/review/SKILL.md",
            "base_dir": "/tmp/review"
        }))
        .expect("skill fixture")
    }

    #[test]
    fn prompt_dispatch_expands_the_frozen_template() {
        let mut skill = fixture();
        skill.command_dispatch = Some("prompt".to_string());
        skill.command_prompt_template = Some("Review $ARGUMENTS carefully".to_string());

        assert_eq!(
            resolve_skill_slash_dispatch(&skill, "  src/lib.rs  "),
            SkillSlashDispatch::ModelTemplate {
                message: "Review src/lib.rs carefully".to_string()
            }
        );
    }

    #[test]
    fn default_dispatch_with_template_does_not_fall_through_to_inline_body() {
        let mut skill = fixture();
        skill.command_prompt_template = Some("Run the checklist".to_string());

        assert_eq!(
            resolve_skill_slash_dispatch(&skill, "src/lib.rs"),
            SkillSlashDispatch::ModelTemplate {
                message: "Run the checklist\n\nUser input:\nsrc/lib.rs".to_string()
            }
        );
    }
}
