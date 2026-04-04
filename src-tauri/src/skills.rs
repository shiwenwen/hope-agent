use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::paths;

// ── Default Constants ────────────────────────────────────────────

const DEFAULT_MAX_SKILLS_IN_PROMPT: usize = 150;
const DEFAULT_MAX_SKILLS_PROMPT_CHARS: usize = 30_000;
const DEFAULT_MAX_SKILL_FILE_BYTES: u64 = 256 * 1024;
const DEFAULT_MAX_CANDIDATES_PER_ROOT: usize = 300;

// ── Cache Version ────────────────────────────────────────────────

static SKILL_CACHE_VERSION: AtomicU64 = AtomicU64::new(0);

/// Bump the global skill cache version, invalidating cached entries.
pub fn bump_skill_version() {
    SKILL_CACHE_VERSION.fetch_add(1, Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn skill_cache_version() -> u64 {
    SKILL_CACHE_VERSION.load(Ordering::Relaxed)
}

// ── Types ─────────────────────────────────────────────────────────

/// Configurable limits for skill prompt generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPromptBudget {
    /// Maximum number of skills to include in the system prompt.
    #[serde(default = "default_max_count")]
    pub max_count: usize,
    /// Maximum total characters for the skills prompt section.
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    /// Maximum size of a SKILL.md file in bytes.
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
    /// Maximum subdirectories to scan per skills root (DoS prevention).
    #[serde(default = "default_max_candidates")]
    pub max_candidates_per_root: usize,
}

fn default_max_count() -> usize {
    DEFAULT_MAX_SKILLS_IN_PROMPT
}
fn default_max_chars() -> usize {
    DEFAULT_MAX_SKILLS_PROMPT_CHARS
}
fn default_max_file_bytes() -> u64 {
    DEFAULT_MAX_SKILL_FILE_BYTES
}
fn default_max_candidates() -> usize {
    DEFAULT_MAX_CANDIDATES_PER_ROOT
}

impl Default for SkillPromptBudget {
    fn default() -> Self {
        Self {
            max_count: DEFAULT_MAX_SKILLS_IN_PROMPT,
            max_chars: DEFAULT_MAX_SKILLS_PROMPT_CHARS,
            max_file_bytes: DEFAULT_MAX_SKILL_FILE_BYTES,
            max_candidates_per_root: DEFAULT_MAX_CANDIDATES_PER_ROOT,
        }
    }
}

/// Environment requirements parsed from SKILL.md frontmatter `requires:` block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillRequires {
    /// Binaries that must exist in PATH (all required — AND logic).
    #[serde(default)]
    pub bins: Vec<String>,
    /// Binaries where at least one must exist (OR logic).
    #[serde(default)]
    pub any_bins: Vec<String>,
    /// Environment variables that must be set (all required).
    #[serde(default)]
    pub env: Vec<String>,
    /// OS identifiers the skill supports, e.g. ["darwin", "linux"].
    /// Empty means all OSes are supported.
    #[serde(default)]
    pub os: Vec<String>,
    /// Config paths that must be truthy (e.g. "webSearch.provider").
    #[serde(default)]
    pub config: Vec<String>,
    /// When true, skip all requirement checks — always eligible.
    #[serde(default)]
    pub always: bool,
    /// Primary env var name that can be satisfied by skill's apiKey config.
    #[serde(default)]
    pub primary_env: Option<String>,
}

/// Installation spec for a skill dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInstallSpec {
    /// Install method: "brew", "node", "go", "uv", "download".
    pub kind: String,
    /// Brew formula name.
    #[serde(default)]
    pub formula: Option<String>,
    /// npm/uv package name.
    #[serde(default)]
    pub package: Option<String>,
    /// Go module path (e.g. "github.com/user/tool@latest").
    #[serde(default, rename = "module")]
    pub go_module: Option<String>,
    /// Binaries to verify after installation.
    #[serde(default)]
    pub bins: Vec<String>,
    /// User-facing label for the install action.
    #[serde(default)]
    pub label: Option<String>,
    /// OS constraints for this install method.
    #[serde(default)]
    pub os: Vec<String>,
}

/// A parsed skill entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    /// Skill identifier (from frontmatter `name`).
    pub name: String,
    /// Human-readable description (from frontmatter `description`).
    pub description: String,
    /// Source category (e.g., "bundled", "managed", "project").
    pub source: String,
    /// Absolute path to the SKILL.md file.
    pub file_path: String,
    /// Directory containing the skill.
    pub base_dir: String,
    /// Environment requirements from frontmatter `requires:` block.
    #[serde(default)]
    pub requires: SkillRequires,
    /// Custom config lookup key (overrides name).
    #[serde(default)]
    pub skill_key: Option<String>,
    /// Whether users can invoke this skill via /command (default: true).
    #[serde(default)]
    pub user_invocable: Option<bool>,
    /// When true, skill is hidden from the model's prompt catalog (default: false).
    #[serde(default)]
    pub disable_model_invocation: Option<bool>,
    /// Command dispatch method (e.g. "tool").
    #[serde(default)]
    pub command_dispatch: Option<String>,
    /// Tool name to bind when command_dispatch is "tool".
    #[serde(default)]
    pub command_tool: Option<String>,
    /// Argument passing mode (e.g. "raw" for direct forwarding to tool).
    #[serde(default)]
    pub command_arg_mode: Option<String>,
    /// Custom argument placeholder for UI display (e.g. "<query>").
    #[serde(default)]
    pub command_arg_placeholder: Option<String>,
    /// Fixed argument choices for UI hints (e.g. ["on", "off"]).
    #[serde(default)]
    pub command_arg_options: Option<Vec<String>>,
    /// Prompt template supporting $ARGUMENTS expansion.
    #[serde(default)]
    pub command_prompt_template: Option<String>,
    /// Installation specs for dependencies.
    #[serde(default)]
    pub install: Vec<SkillInstallSpec>,
    /// Tool restriction: when non-empty, only these tools are available during skill execution.
    /// Parsed from SKILL.md frontmatter `allowed-tools:` field.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Context mode: "fork" runs skill in a sub-agent, "inline" (default) in main conversation.
    /// Parsed from SKILL.md frontmatter `context:` field.
    #[serde(default)]
    pub context_mode: Option<String>,
}

/// Lightweight summary returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub source: String,
    pub base_dir: String,
    pub enabled: bool,
    /// Environment variable names required by this skill (from `requires.env`).
    #[serde(default)]
    pub requires_env: Vec<String>,
    #[serde(default)]
    pub skill_key: Option<String>,
    #[serde(default)]
    pub user_invocable: Option<bool>,
    #[serde(default)]
    pub disable_model_invocation: Option<bool>,
    #[serde(default)]
    pub has_install: bool,
    #[serde(default)]
    pub any_bins: Vec<String>,
    #[serde(default)]
    pub always: bool,
    /// Tool restriction from SKILL.md frontmatter.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Context mode from SKILL.md frontmatter.
    #[serde(default)]
    pub context_mode: Option<String>,
}

/// File metadata inside a skill directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Whether this is a directory.
    pub is_dir: bool,
}

/// Full skill content for detailed view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDetail {
    pub name: String,
    pub description: String,
    pub source: String,
    pub file_path: String,
    pub base_dir: String,
    pub content: String,
    pub enabled: bool,
    /// All files/dirs inside the skill directory.
    pub files: Vec<FileInfo>,
    /// Environment requirements from frontmatter.
    #[serde(default)]
    pub requires: SkillRequires,
    #[serde(default)]
    pub skill_key: Option<String>,
    #[serde(default)]
    pub user_invocable: Option<bool>,
    #[serde(default)]
    pub disable_model_invocation: Option<bool>,
    #[serde(default)]
    pub command_dispatch: Option<String>,
    #[serde(default)]
    pub command_tool: Option<String>,
    #[serde(default)]
    pub install: Vec<SkillInstallSpec>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub context_mode: Option<String>,
}

/// Skill health status for diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillStatusEntry {
    pub name: String,
    pub source: String,
    pub eligible: bool,
    pub disabled: bool,
    pub blocked_by_allowlist: bool,
    #[serde(default)]
    pub missing_bins: Vec<String>,
    #[serde(default)]
    pub missing_any_bins: Vec<String>,
    #[serde(default)]
    pub missing_env: Vec<String>,
    #[serde(default)]
    pub missing_config: Vec<String>,
    pub has_install: bool,
    #[serde(default)]
    pub always: bool,
}

/// Cached skill entries with version tracking.
#[allow(dead_code)]
pub struct SkillCache {
    pub entries: Vec<SkillEntry>,
    pub version: u64,
    pub loaded_at: std::time::Instant,
    pub extra_dirs: Vec<String>,
}

impl SkillCache {
    /// Check if the cache is still valid (30-second TTL + version match).
    #[allow(dead_code)]
    pub fn is_valid(&self, extra_dirs: &[String]) -> bool {
        self.loaded_at.elapsed() < std::time::Duration::from_secs(30)
            && self.version == SKILL_CACHE_VERSION.load(Ordering::Relaxed)
            && self.extra_dirs == extra_dirs
    }
}

// ── Frontmatter Parsing ──────────────────────────────────────────

/// Parsed frontmatter result with all extended fields.
struct ParsedFrontmatter {
    name: String,
    description: String,
    requires: SkillRequires,
    #[allow(dead_code)]
    body: String,
    skill_key: Option<String>,
    user_invocable: Option<bool>,
    disable_model_invocation: Option<bool>,
    command_dispatch: Option<String>,
    command_tool: Option<String>,
    command_arg_mode: Option<String>,
    command_arg_placeholder: Option<String>,
    command_arg_options: Option<Vec<String>>,
    command_prompt_template: Option<String>,
    install: Vec<SkillInstallSpec>,
    allowed_tools: Vec<String>,
    context_mode: Option<String>,
}

/// Extract YAML frontmatter from a SKILL.md file content.
fn parse_frontmatter(content: &str) -> Option<ParsedFrontmatter> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    // Find the closing ---
    let after_opening = &trimmed[3..];
    let end_idx = after_opening.find("\n---")?;
    let yaml_block = &after_opening[..end_idx];
    let body = &after_opening[end_idx + 4..]; // skip \n---

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut skill_key: Option<String> = None;
    let mut user_invocable: Option<bool> = None;
    let mut disable_model_invocation: Option<bool> = None;
    let mut command_dispatch: Option<String> = None;
    let mut command_tool: Option<String> = None;
    let mut command_arg_mode: Option<String> = None;
    let mut command_arg_placeholder: Option<String> = None;
    let mut command_arg_options: Option<Vec<String>> = None;
    let mut command_prompt_template: Option<String> = None;
    let mut allowed_tools: Vec<String> = Vec::new();
    let mut context_mode: Option<String> = None;

    let requires = parse_requires(yaml_block);
    let install = parse_install_specs(yaml_block);

    for line in yaml_block.lines() {
        let line_trimmed = line.trim();
        // Only parse root-level keys (no indentation)
        let indent = line.len() - line.trim_start().len();
        if indent > 0 {
            continue;
        }
        if let Some(rest) = line_trimmed.strip_prefix("name:") {
            name = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed.strip_prefix("description:") {
            description = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("skillKey:")
            .or_else(|| line_trimmed.strip_prefix("skill_key:"))
        {
            skill_key = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("user-invocable:")
            .or_else(|| line_trimmed.strip_prefix("user_invocable:"))
        {
            user_invocable = parse_bool_value(rest.trim());
        } else if let Some(rest) = line_trimmed
            .strip_prefix("disable-model-invocation:")
            .or_else(|| line_trimmed.strip_prefix("disable_model_invocation:"))
        {
            disable_model_invocation = parse_bool_value(rest.trim());
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-dispatch:")
            .or_else(|| line_trimmed.strip_prefix("command_dispatch:"))
        {
            command_dispatch = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-tool:")
            .or_else(|| line_trimmed.strip_prefix("command_tool:"))
        {
            command_tool = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-arg-mode:")
            .or_else(|| line_trimmed.strip_prefix("command_arg_mode:"))
        {
            command_arg_mode = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-arg-placeholder:")
            .or_else(|| line_trimmed.strip_prefix("command_arg_placeholder:"))
        {
            command_arg_placeholder = Some(unquote(rest.trim()));
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-arg-options:")
            .or_else(|| line_trimmed.strip_prefix("command_arg_options:"))
        {
            command_arg_options = parse_inline_string_array(rest.trim());
        } else if let Some(rest) = line_trimmed
            .strip_prefix("command-prompt-template:")
            .or_else(|| line_trimmed.strip_prefix("command_prompt_template:"))
        {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                command_prompt_template = Some(val);
            }
        } else if let Some(rest) = line_trimmed
            .strip_prefix("allowed-tools:")
            .or_else(|| line_trimmed.strip_prefix("allowed_tools:"))
        {
            if let Some(arr) = parse_inline_string_array(rest.trim()) {
                allowed_tools = arr;
            }
        } else if let Some(rest) = line_trimmed.strip_prefix("context:") {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                context_mode = Some(val);
            }
        }
    }

    let name = name.filter(|n| !n.is_empty())?;
    let description = description.unwrap_or_default();

    // For "prompt" dispatch, use body as template if no explicit template was set
    if command_dispatch.as_deref() == Some("prompt") && command_prompt_template.is_none() {
        let body_trimmed = body.trim();
        if !body_trimmed.is_empty() {
            command_prompt_template = Some(body_trimmed.to_string());
        }
    }

    Some(ParsedFrontmatter {
        name,
        description,
        requires,
        body: body.to_string(),
        skill_key,
        user_invocable,
        disable_model_invocation,
        command_dispatch,
        command_tool,
        command_arg_mode,
        command_arg_placeholder,
        command_arg_options,
        command_prompt_template,
        install,
        allowed_tools,
        context_mode,
    })
}

/// Parse a boolean-ish YAML value.
fn parse_bool_value(s: &str) -> Option<bool> {
    let s = unquote(s);
    let lower = s.to_lowercase();
    match lower.as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

/// Parse the `requires:` block from a YAML frontmatter string.
/// Supports both inline arrays `[a, b]` and list style `- item`.
fn parse_requires(yaml_block: &str) -> SkillRequires {
    let mut req = SkillRequires::default();
    let mut in_requires = false;
    let mut current_key = String::new();

    for line in yaml_block.lines() {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        if indent == 0 {
            // Root-level key
            if trimmed == "requires:" || trimmed.starts_with("requires:") {
                in_requires = true;
                // Check for inline value after "requires:"
                if let Some(rest) = trimmed.strip_prefix("requires:") {
                    let rest = rest.trim();
                    // Handle root-level simple keys like "always: true"
                    if !rest.is_empty() && !rest.starts_with('{') {
                        // Not a block, skip
                    }
                }
            } else {
                in_requires = false;
            }
            current_key.clear();
            continue;
        }

        if !in_requires {
            continue;
        }

        if indent >= 2 && indent < 4 {
            // Sub-key of requires (e.g., "bins:", "env:", "os:", "anyBins:", "config:")
            if let Some((key, val)) = trimmed.split_once(':') {
                let key = key.trim();
                let val = val.trim();
                current_key = key.to_string();
                if !val.is_empty() {
                    // Inline array: bins: [git, gh]
                    let items = parse_yaml_inline_list(val);
                    push_requires_items(&mut req, key, items);
                }
            }
        } else if indent >= 4 {
            // List item: - git
            if let Some(item) = trimmed.strip_prefix("- ") {
                let item = unquote(item.trim()).to_string();
                if !item.is_empty() {
                    push_requires_items(&mut req, &current_key, vec![item]);
                }
            }
        }
    }

    // Parse root-level `always:` and `primaryEnv:` (outside requires block)
    for line in yaml_block.lines() {
        let indent = line.len() - line.trim_start().len();
        if indent != 0 {
            continue;
        }
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("always:") {
            if let Some(v) = parse_bool_value(rest.trim()) {
                req.always = v;
            }
        } else if let Some(rest) = trimmed
            .strip_prefix("primaryEnv:")
            .or_else(|| trimmed.strip_prefix("primary_env:"))
        {
            let val = unquote(rest.trim());
            if !val.is_empty() {
                req.primary_env = Some(val);
            }
        }
    }

    req
}

/// Parse a YAML inline list like `[git, gh]` or `["git", "gh"]`.
fn parse_yaml_inline_list(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        inner
            .split(',')
            .map(|item| unquote(item.trim()).to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    }
}

fn push_requires_items(req: &mut SkillRequires, key: &str, items: Vec<String>) {
    match key {
        "bins" => req.bins.extend(items),
        "anyBins" | "any_bins" => req.any_bins.extend(items),
        "env" => req.env.extend(items),
        "os" => req.os.extend(items),
        "config" => req.config.extend(items),
        _ => {}
    }
}

/// Parse the `install:` block from YAML frontmatter.
/// Supports list of install specs with kind/formula/package/module/bins/label/os.
fn parse_install_specs(yaml_block: &str) -> Vec<SkillInstallSpec> {
    let mut specs: Vec<SkillInstallSpec> = Vec::new();
    let mut in_install = false;
    let mut current_spec: Option<InstallSpecBuilder> = None;

    for line in yaml_block.lines() {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        if indent == 0 {
            if trimmed == "install:" {
                in_install = true;
                // Flush any pending spec
                if let Some(builder) = current_spec.take() {
                    if let Some(spec) = builder.build() {
                        specs.push(spec);
                    }
                }
            } else {
                if in_install {
                    // Flush pending spec when leaving install block
                    if let Some(builder) = current_spec.take() {
                        if let Some(spec) = builder.build() {
                            specs.push(spec);
                        }
                    }
                }
                in_install = false;
            }
            continue;
        }

        if !in_install {
            continue;
        }

        // List item start: "- kind: brew" or "- kind: node"
        if indent == 2 && trimmed.starts_with("- ") {
            // Flush previous spec
            if let Some(builder) = current_spec.take() {
                if let Some(spec) = builder.build() {
                    specs.push(spec);
                }
            }
            let rest = &trimmed[2..];
            let mut builder = InstallSpecBuilder::default();
            if let Some((key, val)) = rest.split_once(':') {
                builder.set(key.trim(), val.trim());
            }
            current_spec = Some(builder);
        } else if indent >= 4 {
            // Continuation of current spec
            if let Some(ref mut builder) = current_spec {
                if let Some((key, val)) = trimmed.split_once(':') {
                    builder.set(key.trim(), val.trim());
                }
            }
        }
    }

    // Flush last spec
    if let Some(builder) = current_spec.take() {
        if let Some(spec) = builder.build() {
            specs.push(spec);
        }
    }

    specs
}

#[derive(Default)]
struct InstallSpecBuilder {
    kind: Option<String>,
    formula: Option<String>,
    package: Option<String>,
    go_module: Option<String>,
    bins: Vec<String>,
    label: Option<String>,
    os: Vec<String>,
}

impl InstallSpecBuilder {
    fn set(&mut self, key: &str, val: &str) {
        let val = unquote(val);
        match key {
            "kind" => self.kind = Some(val),
            "formula" => self.formula = Some(val),
            "package" => self.package = Some(val),
            "module" => self.go_module = Some(val),
            "label" => self.label = Some(val),
            "bins" => {
                self.bins = parse_yaml_inline_list(&val)
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            "os" => {
                self.os = parse_yaml_inline_list(&val)
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            _ => {}
        }
    }

    fn build(self) -> Option<SkillInstallSpec> {
        let kind = self.kind?;
        // Validate kind
        match kind.as_str() {
            "brew" | "node" | "go" | "uv" | "download" => {}
            _ => return None,
        }
        Some(SkillInstallSpec {
            kind,
            formula: self.formula,
            package: self.package,
            go_module: self.go_module,
            bins: self.bins,
            label: self.label,
            os: self.os,
        })
    }
}

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

#[derive(Debug, Clone, Default)]
pub struct RequirementsDetail {
    pub eligible: bool,
    pub missing_bins: Vec<String>,
    pub missing_any_bins: Vec<String>,
    pub missing_env: Vec<String>,
    pub missing_config: Vec<String>,
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

/// Remove surrounding quotes from a YAML string value.
/// Parse an inline YAML array like `[opt1, opt2, "opt 3"]` into `Some(Vec<String>)`.
/// Returns `None` if the input doesn't look like an array.
fn parse_inline_string_array(s: &str) -> Option<Vec<String>> {
    let s = s.trim();
    let inner = s.strip_prefix('[')?.strip_suffix(']')?;
    let items: Vec<String> = inner
        .split(',')
        .map(|item| unquote(item.trim()))
        .filter(|item| !item.is_empty())
        .collect();
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ── Path Utilities ───────────────────────────────────────────────

/// Compact a file path by replacing the home directory prefix with `~`.
/// Saves ~5-6 tokens per skill path in the prompt.
fn compact_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        let home_ref = home_str.as_ref();
        if path.starts_with(home_ref) {
            let suffix = &path[home_ref.len()..];
            if suffix.starts_with('/') || suffix.starts_with('\\') {
                return format!("~{}", suffix);
            }
        }
    }
    path.to_string()
}

// ── Discovery ────────────────────────────────────────────────────

/// Discover skills from a single directory.
/// Each immediate subdirectory containing a SKILL.md is treated as a skill.
/// Also detects nested `skills/` subdirectories for recursive scan.
fn load_skills_from_dir(dir: &Path, source: &str, budget: &SkillPromptBudget) -> Vec<SkillEntry> {
    let mut entries = Vec::new();

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    let mut candidate_count = 0;

    for entry in read_dir.flatten() {
        candidate_count += 1;
        if candidate_count > budget.max_candidates_per_root {
            app_warn!(
                "skills",
                "loader",
                "Reached max candidates limit ({}) for directory: {}",
                budget.max_candidates_per_root,
                dir.display()
            );
            break;
        }

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if skill_md.is_file() {
            // Direct skill directory
            if let Some(skill) = load_single_skill(&skill_md, &path, source, budget.max_file_bytes)
            {
                entries.push(skill);
            }
        } else {
            // Check for nested skills/ subdirectory
            let nested_skills = path.join("skills");
            if nested_skills.is_dir() {
                let nested = load_skills_from_dir(&nested_skills, source, budget);
                entries.extend(nested);
            }
        }
    }

    entries
}

/// Load a single skill from its SKILL.md file.
fn load_single_skill(
    skill_md: &Path,
    skill_dir: &Path,
    source: &str,
    max_file_bytes: u64,
) -> Option<SkillEntry> {
    // Check file size
    if let Ok(meta) = std::fs::metadata(skill_md) {
        if meta.len() > max_file_bytes {
            app_warn!(
                "skills",
                "loader",
                "Skipping oversized SKILL.md: {} ({} bytes)",
                skill_md.display(),
                meta.len()
            );
            return None;
        }
    }

    let content = match std::fs::read_to_string(skill_md) {
        Ok(c) => c,
        Err(e) => {
            app_warn!(
                "skills",
                "loader",
                "Failed to read {}: {}",
                skill_md.display(),
                e
            );
            return None;
        }
    };

    let parsed = parse_frontmatter(&content)?;

    Some(SkillEntry {
        name: parsed.name,
        description: parsed.description,
        source: source.to_string(),
        file_path: skill_md.to_string_lossy().to_string(),
        base_dir: skill_dir.to_string_lossy().to_string(),
        requires: parsed.requires,
        skill_key: parsed.skill_key,
        user_invocable: parsed.user_invocable,
        disable_model_invocation: parsed.disable_model_invocation,
        command_dispatch: parsed.command_dispatch,
        command_tool: parsed.command_tool,
        command_arg_mode: parsed.command_arg_mode,
        command_arg_placeholder: parsed.command_arg_placeholder,
        command_arg_options: parsed.command_arg_options,
        command_prompt_template: parsed.command_prompt_template,
        install: parsed.install,
        allowed_tools: parsed.allowed_tools,
        context_mode: parsed.context_mode,
    })
}

/// Load all skills from all configured sources.
///
/// Sources (lowest → highest precedence):
/// 1. Extra directories (user-imported, lowest)
/// 2. Managed skills (~/.opencomputer/skills/)
/// 3. Project-specific skills (.opencomputer/skills/ in cwd, highest)
pub fn load_all_skills_with_extra(extra_dirs: &[String]) -> Vec<SkillEntry> {
    load_all_skills_with_budget(extra_dirs, &SkillPromptBudget::default())
}

/// Load all skills with configurable budget limits.
pub fn load_all_skills_with_budget(
    extra_dirs: &[String],
    budget: &SkillPromptBudget,
) -> Vec<SkillEntry> {
    let mut all: Vec<SkillEntry> = Vec::new();

    // Collect from all sources (lowest precedence first)
    let mut sources: Vec<(PathBuf, String)> = Vec::new();

    // 1. Extra directories (user-imported)
    for dir in extra_dirs {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            // Use last path component as label
            let label = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| dir.clone());
            sources.push((path, label));
        }
    }

    // 2. Managed skills: ~/.opencomputer/skills/
    if let Ok(dir) = paths::skills_dir() {
        sources.push((dir, "managed".to_string()));
    }

    // 3. Project-specific skills: .opencomputer/skills/ relative to cwd
    if let Ok(cwd) = std::env::current_dir() {
        let project_skills = cwd.join(".opencomputer").join("skills");
        if project_skills.is_dir() {
            sources.push((project_skills, "project".to_string()));
        }
    }

    // Higher-precedence sources override lower ones
    for (dir, source) in &sources {
        let entries = load_skills_from_dir(dir, source, budget);
        for entry in entries {
            // Remove any previous entry with the same name (lower precedence)
            all.retain(|e| e.name != entry.name);
            all.push(entry);
        }
    }

    // Sort alphabetically
    all.sort_by(|a, b| a.name.cmp(&b.name));

    all
}

/// Convenience wrapper: load all skills without extra dirs.
#[allow(dead_code)]
pub fn load_all_skills() -> Vec<SkillEntry> {
    load_all_skills_with_extra(&[])
}

// ── Prompt Generation ────────────────────────────────────────────

/// Build the skills section of the system prompt with lazy-load pattern.
///
/// Three-tier progressive degradation:
/// 1. Full format: `- name: description (read: ~/path/SKILL.md)`
/// 2. Compact format: `- name (read: ~/path/SKILL.md)` — when full exceeds budget
/// 3. Truncated: binary-search largest prefix that fits compact budget
///
/// Skills with `disable_model_invocation == true` are excluded from the prompt.
/// Disabled skills and skills failing env_check are also excluded.
/// `allow_bundled` restricts which bundled skills are included (empty = all allowed).
pub fn build_skills_prompt(
    skills: &[SkillEntry],
    disabled: &[String],
    env_check: bool,
    skill_env: &HashMap<String, HashMap<String, String>>,
    budget: &SkillPromptBudget,
    allow_bundled: &[String],
) -> String {
    let active: Vec<&SkillEntry> = skills
        .iter()
        .filter(|s| !disabled.contains(&s.name))
        // Filter by invocation policy: hide from model if disabled
        .filter(|s| s.disable_model_invocation != Some(true))
        // Bundled allowlist
        .filter(|s| {
            if allow_bundled.is_empty() || s.source != "bundled" {
                return true;
            }
            let key = s.skill_key.as_deref().unwrap_or(&s.name);
            allow_bundled.iter().any(|a| a == key || a == &s.name)
        })
        .filter(|s| !env_check || check_requirements(&s.requires, skill_env.get(&s.name)))
        .collect();

    if active.is_empty() {
        return String::new();
    }

    let max_count = budget.max_count.min(active.len());
    let active = &active[..max_count];

    // Header
    let header = "\n\nThe following skills provide specialized instructions for specific tasks.\n\
        Use the `read` tool to load a skill's file when the task matches its name.\n\
        When a skill file references a relative path, resolve it against the skill \
        directory (parent of SKILL.md) and use that absolute path in tool commands.\n\
        Only read the skill most relevant to the current task — do not read more than one skill up front.";

    // Try full format first
    let full_lines: Vec<String> = active
        .iter()
        .map(|s| {
            format!(
                "- {}: {} (read: {})",
                s.name,
                s.description,
                compact_path(&s.file_path)
            )
        })
        .collect();

    let full_text = format!("{}\n{}", header, full_lines.join("\n"));

    if full_text.len() <= budget.max_chars {
        return full_text;
    }

    // Fall back to compact format (no descriptions)
    let compact_lines: Vec<String> = active
        .iter()
        .map(|s| format!("- {} (read: {})", s.name, compact_path(&s.file_path)))
        .collect();

    let compact_text = format!("{}\n{}", header, compact_lines.join("\n"));

    if compact_text.len() <= budget.max_chars {
        let warning = format!(
            "\n\n\u{26a0}\u{fe0f} Skills catalog using compact format (descriptions omitted). {} skills available.",
            active.len()
        );
        return format!("{}{}", compact_text, warning);
    }

    // Binary search for largest prefix that fits
    let mut lo: usize = 0;
    let mut hi: usize = compact_lines.len();

    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate = format!("{}\n{}", header, compact_lines[..mid].join("\n"));
        // Reserve space for truncation warning (~120 chars)
        if candidate.len() + 120 <= budget.max_chars {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    let truncated = if lo > 0 {
        format!(
            "{}\n{}\n\n\u{26a0}\u{fe0f} Skills truncated: showing {} of {} (compact format, descriptions omitted).",
            header,
            compact_lines[..lo].join("\n"),
            lo,
            active.len()
        )
    } else {
        // Even one skill doesn't fit — just show the header
        header.to_string()
    };

    truncated
}

// ── Skill-Slash Command Integration ──────────────────────────────

/// Build slash command definitions from user-invocable skills.
/// Returns skill entries that should be registered as slash commands.
pub fn get_invocable_skills(extra_dirs: &[String], disabled: &[String]) -> Vec<SkillEntry> {
    let skills = load_all_skills_with_extra(extra_dirs);
    skills
        .into_iter()
        .filter(|s| !disabled.contains(&s.name))
        .filter(|s| s.user_invocable != Some(false))
        .collect()
}

/// Normalize a skill name into a valid slash command name.
/// - Lowercase, non-alphanumeric → `_`, truncate to 32 chars, deduplicate underscores.
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

/// Scan a skill directory for all files/subdirectories.
fn scan_skill_files(base_dir: &str) -> Vec<FileInfo> {
    let mut files = Vec::new();
    let dir = Path::new(base_dir);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.path().is_dir();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.push(FileInfo { name, size, is_dir });
        }
    }
    // Sort: directories first, then alphabetically
    files.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    files
}

/// Get the full content of a specific skill's SKILL.md.
pub fn get_skill_content(
    name: &str,
    extra_dirs: &[String],
    disabled: &[String],
) -> Option<SkillDetail> {
    let skills = load_all_skills_with_extra(extra_dirs);
    let entry = skills.into_iter().find(|s| s.name == name)?;

    let content = std::fs::read_to_string(&entry.file_path).ok()?;

    let files = scan_skill_files(&entry.base_dir);
    let enabled = !disabled.contains(&entry.name);

    Some(SkillDetail {
        name: entry.name,
        description: entry.description,
        source: entry.source,
        file_path: entry.file_path,
        base_dir: entry.base_dir,
        content,
        enabled,
        files,
        requires: entry.requires,
        skill_key: entry.skill_key,
        user_invocable: entry.user_invocable,
        disable_model_invocation: entry.disable_model_invocation,
        command_dispatch: entry.command_dispatch,
        command_tool: entry.command_tool,
        install: entry.install,
        allowed_tools: entry.allowed_tools,
        context_mode: entry.context_mode,
    })
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(name: &str, desc: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            description: desc.to_string(),
            source: "managed".to_string(),
            file_path: format!("/tmp/{}/SKILL.md", name),
            base_dir: format!("/tmp/{}", name),
            requires: SkillRequires::default(),
            skill_key: None,
            user_invocable: None,
            disable_model_invocation: None,
            command_dispatch: None,
            command_tool: None,
            install: vec![],
            allowed_tools: vec![],
            context_mode: None,
        }
    }

    fn make_skill_with_path(name: &str, desc: &str, path: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            description: desc.to_string(),
            source: "managed".to_string(),
            file_path: path.to_string(),
            base_dir: format!("/tmp/{}", name),
            requires: SkillRequires::default(),
            skill_key: None,
            user_invocable: None,
            disable_model_invocation: None,
            command_dispatch: None,
            command_tool: None,
            install: vec![],
            allowed_tools: vec![],
            context_mode: None,
        }
    }

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = r#"---
name: github
description: "GitHub operations via gh CLI"
---

# GitHub Skill

Use the gh CLI.
"#;
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "github");
        assert_eq!(parsed.description, "GitHub operations via gh CLI");
        assert!(parsed.body.contains("# GitHub Skill"));
        assert!(parsed.skill_key.is_none());
        assert!(parsed.user_invocable.is_none());
    }

    #[test]
    fn test_parse_frontmatter_extended() {
        let content = r#"---
name: slack
description: "Slack messaging"
skillKey: slack-custom
user-invocable: true
disable-model-invocation: false
command-dispatch: tool
command-tool: slack_send
---

Body
"#;
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "slack");
        assert_eq!(parsed.skill_key.as_deref(), Some("slack-custom"));
        assert_eq!(parsed.user_invocable, Some(true));
        assert_eq!(parsed.disable_model_invocation, Some(false));
        assert_eq!(parsed.command_dispatch.as_deref(), Some("tool"));
        assert_eq!(parsed.command_tool.as_deref(), Some("slack_send"));
    }

    #[test]
    fn test_parse_frontmatter_unquoted() {
        let content = "---\nname: my-skill\ndescription: A simple skill\n---\nBody here";
        let parsed = parse_frontmatter(content).unwrap();
        assert_eq!(parsed.name, "my-skill");
        assert_eq!(parsed.description, "A simple skill");
    }

    #[test]
    fn test_parse_frontmatter_missing_name() {
        let content = "---\ndescription: No name\n---\nBody";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "Just regular markdown";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_requires_inline() {
        let yaml = "name: git\ndescription: d\nrequires:\n  bins: [git, gh]\n  env: [GITHUB_TOKEN]\n  os: [darwin, linux]\n";
        let req = parse_requires(yaml);
        assert_eq!(req.bins, vec!["git", "gh"]);
        assert_eq!(req.env, vec!["GITHUB_TOKEN"]);
        assert_eq!(req.os, vec!["darwin", "linux"]);
    }

    #[test]
    fn test_parse_requires_list_style() {
        let yaml = "name: git\ndescription: d\nrequires:\n  bins:\n    - git\n    - gh\n  env:\n    - GITHUB_TOKEN\n";
        let req = parse_requires(yaml);
        assert_eq!(req.bins, vec!["git", "gh"]);
        assert_eq!(req.env, vec!["GITHUB_TOKEN"]);
    }

    #[test]
    fn test_parse_requires_any_bins() {
        let yaml = "name: test\ndescription: d\nrequires:\n  anyBins: [rg, grep]\n  bins: [git]\n";
        let req = parse_requires(yaml);
        assert_eq!(req.bins, vec!["git"]);
        assert_eq!(req.any_bins, vec!["rg", "grep"]);
    }

    #[test]
    fn test_parse_requires_always() {
        let yaml = "name: test\ndescription: d\nalways: true\nrequires:\n  bins: [nonexistent_binary_xyz]\n";
        let req = parse_requires(yaml);
        assert!(req.always);
        assert_eq!(req.bins, vec!["nonexistent_binary_xyz"]);
    }

    #[test]
    fn test_parse_requires_primary_env() {
        let yaml =
            "name: test\ndescription: d\nprimaryEnv: MY_API_KEY\nrequires:\n  env: [MY_API_KEY]\n";
        let req = parse_requires(yaml);
        assert_eq!(req.primary_env.as_deref(), Some("MY_API_KEY"));
        assert_eq!(req.env, vec!["MY_API_KEY"]);
    }

    #[test]
    fn test_parse_requires_config() {
        let yaml = "name: test\ndescription: d\nrequires:\n  config: [webSearch.provider]\n";
        let req = parse_requires(yaml);
        assert_eq!(req.config, vec!["webSearch.provider"]);
    }

    #[test]
    fn test_parse_install_specs() {
        let yaml = r#"name: test
description: d
install:
  - kind: brew
    formula: gh
    bins: [gh]
    label: "Install GitHub CLI"
  - kind: node
    package: "@anthropic-ai/sdk"
"#;
        let specs = parse_install_specs(yaml);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].kind, "brew");
        assert_eq!(specs[0].formula.as_deref(), Some("gh"));
        assert_eq!(specs[0].bins, vec!["gh"]);
        assert_eq!(specs[0].label.as_deref(), Some("Install GitHub CLI"));
        assert_eq!(specs[1].kind, "node");
        assert_eq!(specs[1].package.as_deref(), Some("@anthropic-ai/sdk"));
    }

    #[test]
    fn test_build_skills_prompt_empty() {
        assert_eq!(
            build_skills_prompt(
                &[],
                &[],
                false,
                &HashMap::new(),
                &SkillPromptBudget::default(),
                &[]
            ),
            ""
        );
    }

    #[test]
    fn test_build_skills_prompt_full_format() {
        let skills = vec![make_skill_with_path(
            "github",
            "GitHub ops",
            "/home/user/skills/github/SKILL.md",
        )];
        let prompt = build_skills_prompt(
            &skills,
            &[],
            false,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &[],
        );
        assert!(prompt.contains("- github: GitHub ops (read:"));
        assert!(prompt.contains("SKILL.md"));
        assert!(prompt.contains("read"));
    }

    #[test]
    fn test_build_skills_prompt_disabled() {
        let skills = vec![make_skill("github", "GitHub ops")];
        let prompt = build_skills_prompt(
            &skills,
            &["github".to_string()],
            false,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &[],
        );
        assert_eq!(prompt, "");
    }

    #[test]
    fn test_build_skills_prompt_disable_model_invocation() {
        let mut skill = make_skill("github", "GitHub ops");
        skill.disable_model_invocation = Some(true);
        let skills = vec![skill];
        let prompt = build_skills_prompt(
            &skills,
            &[],
            false,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &[],
        );
        assert_eq!(prompt, "");
    }

    #[test]
    fn test_build_skills_prompt_compact_fallback() {
        // Create skills that would exceed a tiny budget in full format
        let mut skills = Vec::new();
        for i in 0..50 {
            skills.push(make_skill_with_path(
                &format!("skill_{}", i),
                &format!("A very long description for skill number {} that takes up lots of space in the prompt", i),
                &format!("/home/user/skills/skill_{}/SKILL.md", i),
            ));
        }
        let budget = SkillPromptBudget {
            max_count: 150,
            max_chars: 2000, // Very small budget to force compact
            max_file_bytes: DEFAULT_MAX_SKILL_FILE_BYTES,
            max_candidates_per_root: DEFAULT_MAX_CANDIDATES_PER_ROOT,
        };
        let prompt = build_skills_prompt(&skills, &[], false, &HashMap::new(), &budget, &[]);
        // Should either use compact format or be truncated
        assert!(prompt.contains("read:") || prompt.is_empty());
    }

    #[test]
    fn test_build_skills_prompt_bundled_allowlist() {
        let mut skill1 = make_skill("github", "GitHub ops");
        skill1.source = "bundled".to_string();
        let mut skill2 = make_skill("slack", "Slack ops");
        skill2.source = "bundled".to_string();
        let skill3 = make_skill("custom", "Custom ops"); // source: "managed"
        let skills = vec![skill1, skill2, skill3];

        // Only allow "github" from bundled
        let prompt = build_skills_prompt(
            &skills,
            &[],
            false,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &["github".to_string()],
        );
        assert!(prompt.contains("github"));
        assert!(!prompt.contains("slack")); // blocked by allowlist
        assert!(prompt.contains("custom")); // non-bundled, always allowed
    }

    #[test]
    fn test_build_skills_prompt_env_check_no_requires() {
        // Skill with no requires should always pass env_check
        let skills = vec![make_skill("basic", "A basic skill")];
        let prompt = build_skills_prompt(
            &skills,
            &[],
            true,
            &HashMap::new(),
            &SkillPromptBudget::default(),
            &[],
        );
        assert!(prompt.contains("basic"));
    }

    #[test]
    fn test_check_requirements_empty() {
        // Empty requirements always pass
        assert!(check_requirements(&SkillRequires::default(), None));
    }

    #[test]
    fn test_check_requirements_always() {
        let req = SkillRequires {
            always: true,
            bins: vec!["nonexistent_binary_abc_xyz".to_string()],
            ..Default::default()
        };
        // always=true should pass even with nonexistent binary
        assert!(check_requirements(&req, None));
    }

    #[test]
    fn test_check_requirements_any_bins_pass() {
        // git should exist on most systems
        let req = SkillRequires {
            any_bins: vec!["nonexistent_abc_xyz".to_string(), "sh".to_string()],
            ..Default::default()
        };
        // "sh" should exist, so OR logic passes
        assert!(check_requirements(&req, None));
    }

    #[test]
    fn test_check_requirements_any_bins_fail() {
        let req = SkillRequires {
            any_bins: vec![
                "nonexistent_abc_1".to_string(),
                "nonexistent_abc_2".to_string(),
            ],
            ..Default::default()
        };
        assert!(!check_requirements(&req, None));
    }

    #[test]
    fn test_check_requirements_wrong_os() {
        let req = SkillRequires {
            os: vec!["nonexistent-os-xyz".to_string()],
            ..Default::default()
        };
        assert!(!check_requirements(&req, None));
    }

    #[test]
    fn test_check_requirements_with_configured_env() {
        let req = SkillRequires {
            env: vec!["MY_TEST_KEY_XYZ".to_string()],
            ..Default::default()
        };
        // Without configured env, should fail (assuming MY_TEST_KEY_XYZ is not set)
        assert!(!check_requirements(&req, None));
        // With configured env, should pass
        let mut configured = HashMap::new();
        configured.insert("MY_TEST_KEY_XYZ".to_string(), "some-value".to_string());
        assert!(check_requirements(&req, Some(&configured)));
        // Empty value should still fail
        configured.insert("MY_TEST_KEY_XYZ".to_string(), String::new());
        assert!(!check_requirements(&req, Some(&configured)));
    }

    #[test]
    fn test_check_requirements_primary_env() {
        let req = SkillRequires {
            env: vec!["MY_API_KEY".to_string()],
            primary_env: Some("MY_API_KEY".to_string()),
            ..Default::default()
        };
        // With apiKey configured via __apiKey__, primary_env should be satisfied
        let mut configured = HashMap::new();
        configured.insert("__apiKey__".to_string(), "sk-test-123".to_string());
        assert!(check_requirements(&req, Some(&configured)));
    }

    #[test]
    fn test_compact_path() {
        // Can't test exact home dir, but test the no-change case
        assert_eq!(compact_path("/usr/local/bin/tool"), "/usr/local/bin/tool");
        // Path without home prefix stays unchanged
        assert_eq!(compact_path("/etc/config"), "/etc/config");
    }

    #[test]
    fn test_normalize_skill_command_name() {
        assert_eq!(normalize_skill_command_name("github"), "github");
        assert_eq!(normalize_skill_command_name("my-skill"), "my_skill");
        assert_eq!(
            normalize_skill_command_name("My Cool Skill!"),
            "my_cool_skill"
        );
        assert_eq!(normalize_skill_command_name("---test---"), "test");
        assert_eq!(normalize_skill_command_name(""), "skill");
        // Long name truncation
        let long = "a".repeat(50);
        assert_eq!(normalize_skill_command_name(&long).len(), 32);
    }

    #[test]
    fn test_mask_value() {
        assert_eq!(mask_value(""), "");
        assert_eq!(mask_value("short"), "****");
        assert_eq!(mask_value("12345678"), "****");
        assert_eq!(mask_value("123456789"), "1234...6789");
        assert_eq!(mask_value("sk-abcdefghijklmnop"), "sk-a...mnop");
    }

    #[test]
    fn test_is_masked_value() {
        assert!(is_masked_value("****"));
        assert!(is_masked_value("1234...6789"));
        assert!(!is_masked_value("real-value"));
        assert!(!is_masked_value(""));
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"hello\""), "hello");
        assert_eq!(unquote("'world'"), "world");
        assert_eq!(unquote("plain"), "plain");
    }

    #[test]
    fn test_check_requirements_detail() {
        let req = SkillRequires {
            bins: vec!["nonexistent_bin_xyz".to_string()],
            any_bins: vec!["nonexistent_a".to_string(), "nonexistent_b".to_string()],
            env: vec!["NONEXISTENT_ENV_XYZ".to_string()],
            ..Default::default()
        };
        let detail = check_requirements_detail(&req, None);
        assert!(!detail.eligible);
        assert_eq!(detail.missing_bins, vec!["nonexistent_bin_xyz"]);
        assert_eq!(
            detail.missing_any_bins,
            vec!["nonexistent_a", "nonexistent_b"]
        );
        assert_eq!(detail.missing_env, vec!["NONEXISTENT_ENV_XYZ"]);
    }

    #[test]
    fn test_check_requirements_detail_always() {
        let req = SkillRequires {
            always: true,
            bins: vec!["nonexistent_bin_xyz".to_string()],
            ..Default::default()
        };
        let detail = check_requirements_detail(&req, None);
        assert!(detail.eligible);
        assert!(detail.missing_bins.is_empty());
    }

    #[test]
    fn test_health_check() {
        let skills = vec![
            make_skill("ok-skill", "passes"),
            make_skill("disabled-skill", "disabled"),
        ];
        let disabled = vec!["disabled-skill".to_string()];
        let statuses = check_all_skills_status(&skills, &disabled, false, &HashMap::new(), &[]);
        assert_eq!(statuses.len(), 2);
        assert!(statuses[0].eligible);
        assert!(!statuses[0].disabled);
        assert!(!statuses[1].eligible);
        assert!(statuses[1].disabled);
    }

    #[test]
    fn test_parse_bool_value() {
        assert_eq!(parse_bool_value("true"), Some(true));
        assert_eq!(parse_bool_value("yes"), Some(true));
        assert_eq!(parse_bool_value("false"), Some(false));
        assert_eq!(parse_bool_value("no"), Some(false));
        assert_eq!(parse_bool_value("invalid"), None);
    }

    #[test]
    fn test_skill_cache_version() {
        let v1 = skill_cache_version();
        bump_skill_version();
        let v2 = skill_cache_version();
        assert!(v2 > v1);
    }
}
