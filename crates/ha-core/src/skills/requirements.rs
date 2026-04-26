use std::collections::HashMap;

use super::types::*;

// ── Requirements Checking ────────────────────────────────────────

/// Check whether a skill's requirements are satisfied in the current environment.
/// `configured_env` provides user-configured env var overrides from the settings UI.
pub fn check_requirements(
    req: &SkillRequires,
    configured_env: Option<&HashMap<String, String>>,
) -> bool {
    // always flag: skip all checks
    if req.always {
        return true;
    }

    // Check OS constraint
    if !req.os.is_empty() {
        let current = std::env::consts::OS; // "macos", "linux", "windows"
        let ok = req.os.iter().any(|os| {
            let os = os.as_str();
            os == current
                || (os == "darwin" && current == "macos")
                || (os == "mac" && current == "macos")
                || (os == "win32" && current == "windows")
        });
        if !ok {
            return false;
        }
    }

    // Check binaries in PATH (AND logic: all must exist)
    for bin in &req.bins {
        if !binary_in_path(bin) {
            return false;
        }
    }

    // Check any_bins (OR logic: at least one must exist)
    if !req.any_bins.is_empty() {
        if !req.any_bins.iter().any(|b| binary_in_path(b)) {
            return false;
        }
    }

    // Check environment variables: user-configured values take priority over system env
    for key in &req.env {
        let has_configured = configured_env
            .and_then(|m| m.get(key))
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        // primary_env: if this key matches primary_env and apiKey is configured, it's satisfied
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
            return false;
        }
    }

    true
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
            detail.eligible = false;
        }
    }

    // bins (AND)
    for bin in &req.bins {
        if !binary_in_path(bin) {
            detail.missing_bins.push(bin.clone());
            detail.eligible = false;
        }
    }

    // any_bins (OR)
    if !req.any_bins.is_empty() {
        if !req.any_bins.iter().any(|b| binary_in_path(b)) {
            detail.missing_any_bins = req.any_bins.clone();
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
            detail.eligible = false;
        }
    }

    detail
}

/// Mask a secret value for frontend display.
/// Same pattern as ProviderConfig::masked().
pub fn mask_value(v: &str) -> String {
    if v.len() > 8 {
        format!("{}...{}", &v[..4], &v[v.len() - 4..])
    } else if !v.is_empty() {
        "****".to_string()
    } else {
        String::new()
    }
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
