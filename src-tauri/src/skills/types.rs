use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

// ── Default Constants ────────────────────────────────────────────

pub(super) const DEFAULT_MAX_SKILLS_IN_PROMPT: usize = 150;
pub(super) const DEFAULT_MAX_SKILLS_PROMPT_CHARS: usize = 30_000;
pub(super) const DEFAULT_MAX_SKILL_FILE_BYTES: u64 = 256 * 1024;
pub(super) const DEFAULT_MAX_CANDIDATES_PER_ROOT: usize = 300;

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

pub(super) fn skill_cache_version_raw() -> u64 {
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
            && self.version == skill_cache_version_raw()
            && self.extra_dirs == extra_dirs
    }
}

/// Detailed requirements check result.
#[derive(Debug, Clone, Default)]
pub struct RequirementsDetail {
    pub eligible: bool,
    pub missing_bins: Vec<String>,
    pub missing_any_bins: Vec<String>,
    pub missing_env: Vec<String>,
    pub missing_config: Vec<String>,
}
