use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

// 类型已下沉 ha-config-schema，原地再导出保持 `crate::skills::SkillPromptBudget` 路径不变。
// （原先那条 `#[cfg(test)] pub(super) use` 的两个上限常量只服务 skills 模块内的
// prompt 预算测试，随 `tests.rs` 迁入 ha-skills 后改由那边直接引 schema。）
pub use ha_config_schema::skills::SkillPromptBudget;

// ── Cache Version ────────────────────────────────────────────────

static SKILL_CACHE_VERSION: AtomicU64 = AtomicU64::new(0);

/// Bump the global skill cache version, invalidating cached entries.
///
/// Also emits `skills:catalog_changed` on the global EventBus so passive
/// observers (e.g. `ChannelRegistry::sync_commands_for_all` re-syncing
/// Telegram / Discord bot menus) can react. The emission is best-effort —
/// no bus during early init = silent skip; subscribers are responsible for
/// debouncing if they expect storms (e.g. bulk skill imports).
pub fn bump_skill_version() {
    SKILL_CACHE_VERSION.fetch_add(1, Ordering::Relaxed);
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit("skills:catalog_changed", serde_json::Value::Null);
    }
}

#[allow(dead_code)]
pub fn skill_cache_version() -> u64 {
    SKILL_CACHE_VERSION.load(Ordering::Relaxed)
}

pub(super) fn skill_cache_version_raw() -> u64 {
    SKILL_CACHE_VERSION.load(Ordering::Relaxed)
}

// ── Types ─────────────────────────────────────────────────────────

/// Lifecycle status of a skill. `Draft` skills are excluded from discovery /
/// prompt injection until a human reviewer promotes them to `Active`. Auto-
/// generated skills land in `Draft` by default (see `skills::auto_review`).
/// The `Archived` state is reserved for deactivated skills we keep on disk for
/// rollback — also hidden from discovery.
///
/// Serialized as lowercase string in SKILL.md frontmatter (`status: "draft"`).
/// Missing field → `Active` (back-compat with pre-B' skills).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillStatus {
    #[default]
    Active,
    Draft,
    Archived,
}

impl SkillStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Draft => "draft",
            Self::Archived => "archived",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "draft" => Self::Draft,
            "archived" => Self::Archived,
            _ => Self::Active,
        }
    }

    /// Draft/Archived skills are hidden from prompt catalog + tool filtering.
    pub fn is_discoverable(&self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Display-only metadata aggregated from frontmatter. None of these fields
/// affect skill activation or tool dispatch — they exist so the UI can render
/// emoji, tags, license badges, version tooltips, and "related skills" hints.
///
/// Sources (lifted into a single struct so the frontend has one shape):
/// - `version` / `license` / `author` — top-level YAML
/// - `metadata.openclaw.emoji` / `metadata.hermes.emoji` — vendor-namespaced
/// - `metadata.hermes.tags` / `metadata.hermes.related_skills` — vendor-namespaced
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillDisplay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Short license label suitable for a UI badge (the leading SPDX-ish
    /// token, e.g. `MIT` / `Apache-2.0` / `Proprietary`). Derived from
    /// [`Self::license_short`] at parse time so the frontend doesn't
    /// re-parse the verbose `license` string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_label: Option<String>,
    /// True when the license is not a recognized OSS family. Frontend uses
    /// this to surface a warning badge on Anthropic / vendor-restricted
    /// skills. Derived at parse time via [`Self::is_proprietary`].
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_proprietary: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_skills: Vec<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl SkillDisplay {
    /// True when no display-only data was extracted. Lets serializers omit
    /// empty payloads from REST responses.
    pub fn is_empty(&self) -> bool {
        self.emoji.is_none()
            && self.version.is_none()
            && self.license.is_none()
            && self.author.is_none()
            && self.tags.is_empty()
            && self.related_skills.is_empty()
    }

    /// Populate the derived `license_label` and `is_proprietary` fields from
    /// the raw `license` string. Called by the parser once after assembling
    /// the struct, so the frontend gets a flat, computed payload.
    pub fn finalize(&mut self) {
        self.license_label = self.license_short().map(str::to_string);
        self.is_proprietary = self.is_proprietary();
    }

    /// Short label suitable for a UI badge. For SPDX-style identifiers like
    /// `MIT` / `Apache-2.0` this returns the full string; for verbose
    /// proprietary headers (`Proprietary. LICENSE.txt has complete terms`)
    /// it returns the leading word. `None` when no license was declared.
    pub fn license_short(&self) -> Option<&str> {
        self.license.as_deref().map(|s| {
            s.split(|c: char| c == '.' || c.is_whitespace())
                .next()
                .unwrap_or(s)
        })
    }

    /// True when the declared license is **not** a recognized OSS family.
    /// Used to surface a warning badge on Anthropic / vendor-restricted
    /// skills. The match is conservative — anything we don't recognize as
    /// OSS is treated as proprietary. Prefix-match handles SPDX variants
    /// like `BSD-3-Clause`, `Apache-2.0`, `MPL-2.0`.
    pub fn is_proprietary(&self) -> bool {
        let Some(short) = self.license_short() else {
            return false;
        };
        let lower = short.to_ascii_lowercase();
        const OSS_PREFIXES: &[&str] = &[
            "mit",
            "bsd",
            "apache",
            "gpl",
            "lgpl",
            "agpl",
            "mpl",
            "isc",
            "unlicense",
            "cc0",
        ];
        !OSS_PREFIXES.iter().any(|p| lower.starts_with(p))
    }
}

/// Environment requirements parsed from SKILL.md frontmatter `requires:` block.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
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
    /// When true, skip requirement checks. This does not make the skill
    /// locked, undeletable, or always injected.
    #[serde(default)]
    pub always: bool,
    /// Primary env var name that can be satisfied by skill's apiKey config.
    #[serde(default)]
    pub primary_env: Option<String>,
}

impl SkillRequires {
    /// True when no requirement was declared. Used by the frontmatter parser
    /// to decide whether to "lift" a vendor-namespaced `metadata.<vendor>.requires`
    /// block into the top-level — only happens when top-level is empty so we
    /// never silently override an explicit user-set value.
    pub fn is_empty(&self) -> bool {
        self.bins.is_empty()
            && self.any_bins.is_empty()
            && self.env.is_empty()
            && self.os.is_empty()
            && self.config.is_empty()
            && !self.always
            && self.primary_env.is_none()
    }
}

/// Installation spec for a skill dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInstallSpec {
    /// Install method: "brew", "node", "go", or "uv". The historical
    /// "download" value is parsed but rejected by the installer.
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
    /// Alternate slash-command names; alias conflicts are skipped silently.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Human-readable description (from frontmatter `description`).
    pub description: String,
    /// Optional trigger hint. Catalog falls back to `description` when unset;
    /// see `SkillEntry::trigger_text`.
    #[serde(default, rename = "whenToUse", alias = "when_to_use")]
    pub when_to_use: Option<String>,
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
    /// Whether `allowed-tools` was present in frontmatter. This preserves the
    /// security-significant distinction between legacy omission (no added
    /// ceiling) and an explicitly empty list (deny every tool).
    #[serde(default)]
    pub allowed_tools_declared: bool,
    /// Context mode: "fork" runs skill in a sub-agent, "inline" (default) in main conversation.
    /// Parsed from SKILL.md frontmatter `context:` field.
    #[serde(default)]
    pub context_mode: Option<String>,
    /// Sub-agent id to use when this skill is activated with `context: fork`.
    /// Looked up via `agent_loader::load_agent`; invalid ids fall back to the
    /// parent agent at fork time. Parsed from SKILL.md frontmatter `agent:` field.
    #[serde(default)]
    pub agent: Option<String>,
    /// Reasoning / thinking effort forwarded to the provider when this skill
    /// is forked. Parsed from SKILL.md frontmatter `effort:` field.
    #[serde(default)]
    pub effort: Option<String>,
    /// Conditional-activation glob patterns. When present and non-empty the
    /// skill is hidden from the catalog until a file matching one of these
    /// gitignore-style patterns is touched in the current session.
    /// Parsed from SKILL.md frontmatter `paths:` field.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    /// Lifecycle status. `Draft` / `Archived` are excluded from discovery
    /// (and thus prompt + slash + tool_search). Defaults to `Active`.
    #[serde(default)]
    pub status: SkillStatus,
    /// Source hint: "user" for human-authored, "auto-review" for skills
    /// created by the auto-review pipeline. Informational; does not affect
    /// discovery. Missing → "user".
    #[serde(default)]
    pub authored_by: Option<String>,
    /// Free-text rationale recorded when this skill was auto-created or
    /// auto-patched (surfaced in the draft review UI).
    #[serde(default)]
    pub rationale: Option<String>,
    /// Display-only metadata (emoji / tags / version / license / author /
    /// related skills). Aggregated from top-level YAML and vendor-namespaced
    /// `metadata.openclaw` / `metadata.hermes` blocks. Does not affect
    /// activation or dispatch.
    #[serde(default, skip_serializing_if = "SkillDisplay::is_empty")]
    pub display: SkillDisplay,
}

/// Frozen result of resolving one or more explicit composer skill mentions.
/// The prompt payload and execution narrowing are produced from the same
/// entries so they cannot drift across the authority boundary.
#[derive(Debug, Clone, Default)]
pub struct MentionSkillActivation {
    pub content: String,
    pub resolved_names: Vec<String>,
    pub rejected_names: Vec<String>,
    /// Initial explicit-skill ceiling. The chat engine intersects it with any
    /// pre-existing caller ceiling; it can never add tools.
    pub tool_ceiling: SkillToolCeiling,
}

pub const SKILL_DENY_ALL_SENTINEL: &str = "__hope_skill_ceiling_denies_all__";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SkillToolCeiling {
    /// Legacy frontmatter omitted `allowed-tools`; no additional restriction.
    #[default]
    Unspecified,
    /// Frontmatter explicitly declared an empty list.
    DenyAll,
    /// Frontmatter declared a non-empty allowlist.
    Restricted(Vec<String>),
}

impl SkillToolCeiling {
    pub fn execution_filter(&self) -> Vec<String> {
        match self {
            Self::Unspecified => Vec::new(),
            Self::DenyAll => vec![SKILL_DENY_ALL_SENTINEL.to_string()],
            Self::Restricted(tools) => tools.clone(),
        }
    }
}

/// Apply a skill ceiling to the legacy execution-filter representation.
/// Empty `current` means unrestricted; the private impossible-name sentinel
/// represents deny-all. The result is monotonic and never widens `current`.
pub fn narrow_skill_execution_filter(
    current: &mut Vec<String>,
    selected: SkillToolCeiling,
) -> bool {
    let before = current.clone();
    let selected = match selected {
        SkillToolCeiling::Unspecified => return false,
        SkillToolCeiling::DenyAll => vec![SKILL_DENY_ALL_SENTINEL.to_string()],
        SkillToolCeiling::Restricted(tools) => tools,
    };
    if current.is_empty() {
        *current = selected;
    } else if !(current.len() == 1 && current[0] == SKILL_DENY_ALL_SENTINEL) {
        current.retain(|tool| selected.iter().any(|allowed| allowed == tool));
        if current.is_empty() {
            current.push(SKILL_DENY_ALL_SENTINEL.to_string());
        }
    }
    *current != before
}

impl SkillEntry {
    pub fn tool_ceiling(&self) -> SkillToolCeiling {
        if !self.allowed_tools_declared {
            SkillToolCeiling::Unspecified
        } else if self.allowed_tools.is_empty() {
            SkillToolCeiling::DenyAll
        } else {
            let mut tools = crate::mcp::canonicalize_tool_filter_names(&self.allowed_tools);
            tools.sort();
            tools.dedup();
            SkillToolCeiling::Restricted(tools)
        }
    }

    /// Text used to decide "when should this skill trigger" — the catalog
    /// renderer and any future scorer should go through here rather than
    /// branching on `when_to_use.is_some()` at the call site.
    pub fn trigger_text(&self) -> &str {
        match self.when_to_use.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => &self.description,
        }
    }

    /// Every slash-command name this skill responds to, already normalized.
    /// Canonical name first, then aliases. Keeps listing and dispatch in
    /// sync — dispatch iterates, listing flattens.
    pub fn all_command_names(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::once(super::slash::normalize_skill_command_name(&self.name)).chain(
            self.aliases
                .iter()
                .map(|a| super::slash::normalize_skill_command_name(a)),
        )
    }

    /// Does this skill own the given (already-normalized) command name?
    pub fn matches_command(&self, command: &str) -> bool {
        self.all_command_names().any(|n| n == command)
    }

    /// Build a `SkillSummary` from a loaded entry + an enabled/disabled
    /// decision. Centralizes the Tauri/HTTP adapter projections so a new
    /// summary field only needs to be wired up once.
    pub fn to_summary(self, enabled: bool) -> SkillSummary {
        let requires_env = self.requires.env.clone();
        let any_bins = self.requires.any_bins.clone();
        let always = self.requires.always;
        SkillSummary {
            name: self.name,
            description: self.description,
            source: self.source,
            base_dir: self.base_dir,
            enabled,
            requires_env,
            skill_key: self.skill_key,
            user_invocable: self.user_invocable,
            disable_model_invocation: self.disable_model_invocation,
            has_install: !self.install.is_empty(),
            any_bins,
            always,
            allowed_tools: self.allowed_tools,
            allowed_tools_declared: self.allowed_tools_declared,
            context_mode: self.context_mode,
            agent: self.agent,
            effort: self.effort,
            status: self.status,
            authored_by: self.authored_by,
            display: self.display,
        }
    }
}

#[cfg(test)]
mod skill_tool_ceiling_tests {
    use super::{narrow_skill_execution_filter, SkillToolCeiling, SKILL_DENY_ALL_SENTINEL};

    #[test]
    fn ceiling_preserves_omitted_empty_and_restricted_as_three_states() {
        assert!(SkillToolCeiling::Unspecified.execution_filter().is_empty());
        assert_eq!(
            SkillToolCeiling::DenyAll.execution_filter(),
            vec![SKILL_DENY_ALL_SENTINEL.to_string()]
        );
        assert_eq!(
            SkillToolCeiling::Restricted(vec!["read_file".into()]).execution_filter(),
            vec!["read_file"]
        );
    }

    #[test]
    fn narrowing_is_monotonic_and_disjoint_sets_become_deny_all() {
        let mut filter = vec!["read_file".to_string(), "list_dir".to_string()];
        assert!(narrow_skill_execution_filter(
            &mut filter,
            SkillToolCeiling::Restricted(vec!["list_dir".into(), "write_file".into()]),
        ));
        assert_eq!(filter, vec!["list_dir"]);

        assert!(narrow_skill_execution_filter(
            &mut filter,
            SkillToolCeiling::Restricted(vec!["write_file".into()]),
        ));
        assert_eq!(filter, vec![SKILL_DENY_ALL_SENTINEL]);

        assert!(!narrow_skill_execution_filter(
            &mut filter,
            SkillToolCeiling::Unspecified,
        ));
        assert_eq!(filter, vec![SKILL_DENY_ALL_SENTINEL]);
    }
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
    #[serde(default)]
    pub allowed_tools_declared: bool,
    /// Context mode from SKILL.md frontmatter.
    #[serde(default)]
    pub context_mode: Option<String>,
    /// Sub-agent id for `context: fork` skills.
    #[serde(default)]
    pub agent: Option<String>,
    /// Reasoning effort forwarded at fork time.
    #[serde(default)]
    pub effort: Option<String>,
    /// Lifecycle status (see `SkillStatus`).
    #[serde(default)]
    pub status: SkillStatus,
    /// Source hint: "user" / "auto-review".
    #[serde(default)]
    pub authored_by: Option<String>,
    /// Display-only metadata (emoji / tags / version / license / author /
    /// related skills). See [`SkillDisplay`].
    #[serde(default, skip_serializing_if = "SkillDisplay::is_empty")]
    pub display: SkillDisplay,
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
    pub allowed_tools_declared: bool,
    #[serde(default)]
    pub context_mode: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub status: SkillStatus,
    #[serde(default)]
    pub authored_by: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
    /// Display-only metadata (emoji / tags / version / license / author /
    /// related skills). See [`SkillDisplay`].
    #[serde(default, skip_serializing_if = "SkillDisplay::is_empty")]
    pub display: SkillDisplay,
}

/// Skill health status for diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillStatusEntry {
    pub name: String,
    pub source: String,
    pub eligible: bool,
    #[serde(default)]
    pub hard_blocked: bool,
    #[serde(default)]
    pub needs_setup: bool,
    pub disabled: bool,
    pub blocked_by_allowlist: bool,
    #[serde(default)]
    pub current_os: Option<String>,
    #[serde(default)]
    pub supported_os: Vec<String>,
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
    pub hard_blocked: bool,
    pub needs_setup: bool,
    pub current_os: Option<String>,
    pub supported_os: Vec<String>,
    pub missing_bins: Vec<String>,
    pub missing_any_bins: Vec<String>,
    pub missing_env: Vec<String>,
    pub missing_config: Vec<String>,
}

impl RequirementsDetail {
    /// True when the skill may be surfaced to the model. Hard blockers are
    /// things the user cannot fix by installing packages or setting config in
    /// the current environment, such as an unsupported OS.
    pub fn injection_eligible(&self) -> bool {
        !self.hard_blocked
    }
}
