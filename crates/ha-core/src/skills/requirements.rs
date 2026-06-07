use std::collections::HashMap;

use super::types::*;

// ── Requirements Checking ────────────────────────────────────────

/// Check whether a skill's requirements are satisfied in the current environment.
/// `configured_env` provides user-configured env var overrides from the settings UI.
pub fn check_requirements(
    req: &SkillRequires,
    configured_env: Option<&HashMap<String, String>>,
) -> bool {
    check_requirements_detail(req, configured_env).eligible
}

/// Check whether a skill may be surfaced to the model. This is intentionally
/// weaker than [`check_requirements`]: recoverable setup gaps (missing bins,
/// env vars, or app config) still allow injection so the skill can explain how
/// to fix itself when activated. Hard blockers, currently unsupported OS, hide
/// the skill.
pub fn check_requirements_for_injection(
    req: &SkillRequires,
    configured_env: Option<&HashMap<String, String>>,
) -> bool {
    check_requirements_detail(req, configured_env).injection_eligible()
}

/// Detailed requirements check returning missing items.
pub fn check_requirements_detail(
    req: &SkillRequires,
    configured_env: Option<&HashMap<String, String>>,
) -> RequirementsDetail {
    let mut detail = RequirementsDetail::default();

    if req.always {
        detail.eligible = true;
        return detail;
    }

    detail.eligible = true;

    // OS
    if !req.os.is_empty() {
        let current = std::env::consts::OS;
        let ok = req.os.iter().any(|os| {
            let os = os.as_str();
            os == current
                || (os == "darwin" && current == "macos")
                || (os == "mac" && current == "macos")
                || (os == "win32" && current == "windows")
        });
        if !ok {
            detail.hard_blocked = true;
            detail.current_os = Some(current.to_string());
            detail.supported_os = req.os.clone();
            detail.eligible = false;
        }
    }

    // bins (AND)
    for bin in &req.bins {
        if !binary_in_path(bin) {
            detail.missing_bins.push(bin.clone());
            detail.needs_setup = true;
            detail.eligible = false;
        }
    }

    // any_bins (OR)
    if !req.any_bins.is_empty() {
        if !req.any_bins.iter().any(|b| binary_in_path(b)) {
            detail.missing_any_bins = req.any_bins.clone();
            detail.needs_setup = true;
            detail.eligible = false;
        }
    }

    // env
    for key in &req.env {
        let has_configured = configured_env
            .and_then(|m| m.get(key))
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let has_primary = req
            .primary_env
            .as_ref()
            .filter(|pe| pe.as_str() == key)
            .is_some()
            && configured_env
                .and_then(|m| m.get("__apiKey__"))
                .map(|v| !v.is_empty())
                .unwrap_or(false);
        if !has_configured
            && !has_primary
            && std::env::var(key).map(|v| v.is_empty()).unwrap_or(true)
        {
            detail.missing_env.push(key.clone());
            detail.needs_setup = true;
            detail.eligible = false;
        }
    }

    // App config paths are recoverable: users can usually set them in Settings.
    for path in &req.config {
        if !config_path_truthy(path) {
            detail.missing_config.push(path.clone());
            detail.needs_setup = true;
            detail.eligible = false;
        }
    }

    detail
}

/// Resolve the effective skill-env-check switch for a chat Agent. The legacy
/// fallback is the global config switch used outside an AgentDefinition path.
pub fn skill_env_check_enabled_for_agent(agent_id: Option<&str>, fallback: bool) -> bool {
    let Some(agent_id) = agent_id else {
        return fallback;
    };
    crate::agent_loader::load_agent(agent_id)
        .map(|definition| definition.config.capabilities.skill_env_check)
        .unwrap_or(fallback)
}

/// Render a user/model-facing diagnostic for a skill that was discovered but
/// cannot be activated in the current environment.
pub fn format_requirements_diagnostic(skill: &SkillEntry, detail: &RequirementsDetail) -> String {
    let mut lines = Vec::new();
    if detail.hard_blocked {
        let current = detail.current_os.as_deref().unwrap_or(std::env::consts::OS);
        let supported = if detail.supported_os.is_empty() {
            "unspecified".to_string()
        } else {
            detail.supported_os.join(", ")
        };
        lines.push(format!(
            "Skill `{}` cannot run on this operating system.",
            skill.name
        ));
        lines.push(format!(
            "- Current OS: `{}`; supported OS: `{}`.",
            current, supported
        ));
        lines.push(
            "- This is a hard compatibility blocker, not something install/config can fix."
                .to_string(),
        );
    } else {
        lines.push(format!(
            "Skill `{}` needs setup before it can run.",
            skill.name
        ));
    }

    if !detail.missing_bins.is_empty() {
        lines.push(format!(
            "- Missing required binaries in PATH: {}.",
            backtick_join(&detail.missing_bins)
        ));
    }
    if !detail.missing_any_bins.is_empty() {
        lines.push(format!(
            "- Need at least one of these binaries in PATH: {}.",
            backtick_join(&detail.missing_any_bins)
        ));
    }
    if !detail.missing_env.is_empty() {
        lines.push(format!(
            "- Missing environment variables: {}. Configure them in Settings > Skills > `{}` or set them in the process environment.",
            backtick_join(&detail.missing_env),
            skill.name
        ));
    }
    if !detail.missing_config.is_empty() {
        lines.push(format!(
            "- Missing app configuration: {}. Configure these settings before activating the skill.",
            backtick_join(&detail.missing_config)
        ));
    }

    let install_hints = compatible_install_hints(skill);
    if !install_hints.is_empty() {
        lines.push("Install options declared by the skill:".to_string());
        lines.extend(install_hints.into_iter().map(|hint| format!("- {hint}")));
    }

    lines.push("After fixing the missing setup, activate the skill again.".to_string());
    lines.join("\n")
}

fn backtick_join(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn compatible_install_hints(skill: &SkillEntry) -> Vec<String> {
    skill
        .install
        .iter()
        .filter(|spec| install_spec_matches_os(spec))
        .filter_map(|spec| {
            let command = install_command_preview(spec)?;
            let label = spec
                .label
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("Install dependency");
            Some(format!("{label}: `{command}`"))
        })
        .collect()
}

fn install_spec_matches_os(spec: &SkillInstallSpec) -> bool {
    if spec.os.is_empty() {
        return true;
    }
    let current = std::env::consts::OS;
    spec.os.iter().any(|os| os_matches_current(os, current))
}

fn install_command_preview(spec: &SkillInstallSpec) -> Option<String> {
    match spec.kind.as_str() {
        "brew" => spec
            .formula
            .as_ref()
            .map(|formula| format!("brew install {formula}")),
        "node" => spec
            .package
            .as_ref()
            .map(|package| format!("npm install -g {package}")),
        "go" => spec
            .go_module
            .as_ref()
            .map(|module| format!("go install {module}")),
        "uv" => spec
            .package
            .as_ref()
            .map(|package| format!("uv tool install {package}")),
        _ => None,
    }
}

fn os_matches_current(os: &str, current: &str) -> bool {
    os == current
        || (os == "darwin" && current == "macos")
        || (os == "mac" && current == "macos")
        || (os == "win32" && current == "windows")
}

fn config_path_truthy(path: &str) -> bool {
    let Ok(value) = serde_json::to_value(crate::config::cached_config()) else {
        return false;
    };
    let mut cursor = &value;
    for segment in path.split('.').filter(|s| !s.is_empty()) {
        match cursor {
            serde_json::Value::Object(map) => {
                let Some(next) = map.get(segment) else {
                    return false;
                };
                cursor = next;
            }
            _ => return false,
        }
    }
    json_truthy(cursor)
}

fn json_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(v) => *v,
        serde_json::Value::Number(n) => n.as_f64().map(|v| v != 0.0).unwrap_or(true),
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(v) => !v.is_empty(),
        serde_json::Value::Object(v) => !v.is_empty(),
    }
}

/// Mask a secret value for frontend display.
/// Same pattern as ProviderConfig::masked().
pub fn mask_value(v: &str) -> String {
    crate::mask_secret_middle(v, 4, 4)
}

/// Check if a value is a masked placeholder (should not overwrite real value).
pub fn is_masked_value(v: &str) -> bool {
    v == "****" || (v.len() > 7 && v.contains("..."))
}

/// Public wrapper for binary_in_path (used by install command).
pub fn binary_in_path_public(name: &str) -> bool {
    binary_in_path(name)
}

/// Check whether a binary exists anywhere in PATH.
fn binary_in_path(name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return true;
            }
            // Windows: also check .exe
            #[cfg(target_os = "windows")]
            {
                let exe = dir.join(format!("{}.exe", name));
                if exe.is_file() {
                    return true;
                }
            }
        }
    }
    false
}
