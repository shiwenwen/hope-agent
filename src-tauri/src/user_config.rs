use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::paths;

// ── User Config ──────────────────────────────────────────────────

/// Global user configuration, shared across all Agents.
/// Stored at ~/.opencomputer/user.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserConfig {
    /// User's display name / nickname
    #[serde(default)]
    pub name: Option<String>,

    /// Avatar: file path or URL
    #[serde(default)]
    pub avatar: Option<String>,

    /// Gender: "male", "female", or custom text
    #[serde(default)]
    pub gender: Option<String>,

    /// Age
    #[serde(default)]
    pub age: Option<u32>,

    /// Role description, e.g. "全栈开发者"
    #[serde(default)]
    pub role: Option<String>,

    /// IANA timezone, e.g. "Asia/Shanghai"
    #[serde(default)]
    pub timezone: Option<String>,

    /// Preferred language, e.g. "zh-CN", "en"
    #[serde(default)]
    pub language: Option<String>,

    /// AI experience level: "expert", "intermediate", "beginner"
    #[serde(default)]
    pub ai_experience: Option<String>,

    /// Response style: "concise", "detailed", or custom text
    #[serde(default)]
    pub response_style: Option<String>,

    /// Free-form extra info the user wants the AI to know
    #[serde(default)]
    pub custom_info: Option<String>,
}

// ── Persistence ──────────────────────────────────────────────────

/// Load user config from ~/.opencomputer/user.json
/// Returns default if file doesn't exist.
pub fn load_user_config() -> Result<UserConfig> {
    let path = paths::user_config_path()?;
    if !path.exists() {
        return Ok(UserConfig::default());
    }
    let data = std::fs::read_to_string(&path)?;
    let config: UserConfig = serde_json::from_str(&data)?;
    Ok(config)
}

/// Save user config to ~/.opencomputer/user.json
pub fn save_user_config_to_disk(config: &UserConfig) -> Result<()> {
    let path = paths::user_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, data)?;
    Ok(())
}

// ── System Prompt Context ────────────────────────────────────────

/// Helper: push a line if value is non-empty
fn push_if(lines: &mut Vec<String>, label: &str, val: &Option<String>) {
    if let Some(v) = val {
        if !v.is_empty() {
            lines.push(format!("- {}: {}", label, v));
        }
    }
}

/// Build a user context section for injection into the system prompt.
/// Returns None if no meaningful user info is configured.
pub fn build_user_context(config: &UserConfig) -> Option<String> {
    let mut lines = Vec::new();

    push_if(&mut lines, "Name", &config.name);
    push_if(&mut lines, "Gender", &config.gender);
    if let Some(age) = config.age {
        lines.push(format!("- Age: {}", age));
    }
    push_if(&mut lines, "Role", &config.role);
    push_if(&mut lines, "AI experience level", &config.ai_experience);
    push_if(&mut lines, "Preferred language", &config.language);
    push_if(&mut lines, "Timezone", &config.timezone);
    push_if(&mut lines, "Response style", &config.response_style);

    if let Some(info) = &config.custom_info {
        if !info.is_empty() {
            lines.push(format!("- Additional info: {}", info));
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(format!("# User\n\n{}", lines.join("\n")))
    }
}
