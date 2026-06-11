//! Sprite (inspiration mode) configuration — persisted under `AppConfig.sprite`.
//! **Disabled by default**: the sprite makes proactive LLM calls on edit-idle,
//! so nothing runs until the user opts in (settings → knowledge space).

use serde::{Deserialize, Serialize};

fn default_idle_edit_secs() -> u32 {
    6
}
fn default_min_change_chars() -> u32 {
    40
}
fn default_cooldown_secs() -> u64 {
    30
}
fn default_max_per_hour() -> u32 {
    12
}
fn default_periodic_secs() -> u32 {
    120
}
fn default_paste_min_chars() -> u32 {
    180
}
fn default_max_tokens() -> u32 {
    400
}
fn default_timeout_secs() -> u64 {
    20
}
fn default_true() -> bool {
    true
}

/// Per-sense toggles. Each input the sprite fuses into its prompt can be
/// disabled independently (cost / privacy / focus).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteSenses {
    #[serde(default = "default_true")]
    pub doc: bool,
    #[serde(default = "default_true")]
    pub edit: bool,
    #[serde(default = "default_true")]
    pub conversation: bool,
    #[serde(default = "default_true")]
    pub memory: bool,
    #[serde(default = "default_true")]
    pub awareness: bool,
}

impl Default for SpriteSenses {
    fn default() -> Self {
        Self {
            doc: true,
            edit: true,
            conversation: true,
            memory: true,
            awareness: true,
        }
    }
}

/// Per-occasion toggles for *when* the sprite may fire (orthogonal to `senses`,
/// which is *what* it reads). All default on; the cooldown + hourly cap bound the
/// total volume regardless of how many occasions are enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteTriggers {
    /// After a pause in editing (idle + enough changed).
    #[serde(default = "default_true")]
    pub edit_idle: bool,
    /// Shortly after a note is opened, reacting to it as-is (no edit needed).
    #[serde(default = "default_true")]
    pub note_open: bool,
    /// After a turn completes in the knowledge-space chat.
    #[serde(default = "default_true")]
    pub conversation: bool,
    /// Periodically while actively writing (doesn't wait for an idle pause).
    /// Off by default — the most token-hungry occasion; opt-in for power users.
    #[serde(default)]
    pub periodic: bool,
    /// Immediately on a large paste / insert.
    #[serde(default = "default_true")]
    pub paste: bool,
}

impl Default for SpriteTriggers {
    fn default() -> Self {
        Self {
            edit_idle: true,
            note_open: true,
            conversation: true,
            periodic: false,
            paste: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteConfig {
    /// Master switch (default off).
    #[serde(default)]
    pub enabled: bool,
    /// Seconds of editing inactivity before an observation may fire (frontend).
    #[serde(default = "default_idle_edit_secs")]
    pub idle_edit_secs: u32,
    /// Minimum characters changed since the last observation (frontend gate).
    #[serde(default = "default_min_change_chars")]
    pub min_change_chars: u32,
    /// Minimum seconds between LLM calls per session/note (backend gate).
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
    /// Max LLM calls per rolling hour per session/note (backend gate).
    #[serde(default = "default_max_per_hour")]
    pub max_per_session_per_hour: u32,
    /// Interval for the `periodic` trigger (seconds, frontend).
    #[serde(default = "default_periodic_secs")]
    pub periodic_secs: u32,
    /// Single-edit insert size that counts as a paste for the `paste` trigger.
    #[serde(default = "default_paste_min_chars")]
    pub paste_min_chars: u32,
    /// More forthcoming (true) vs. restrained (false) — shapes the persona /
    /// silence bias. Default forthcoming.
    #[serde(default = "default_true")]
    pub proactive: bool,
    #[serde(default)]
    pub triggers: SpriteTriggers,
    #[serde(default)]
    pub senses: SpriteSenses,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for SpriteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_edit_secs: default_idle_edit_secs(),
            min_change_chars: default_min_change_chars(),
            cooldown_secs: default_cooldown_secs(),
            max_per_session_per_hour: default_max_per_hour(),
            periodic_secs: default_periodic_secs(),
            paste_min_chars: default_paste_min_chars(),
            proactive: true,
            triggers: SpriteTriggers::default(),
            senses: SpriteSenses::default(),
            max_tokens: default_max_tokens(),
            timeout_secs: default_timeout_secs(),
        }
    }
}

impl SpriteConfig {
    /// Clamp to sane ranges so a skill/HTTP write can't persist values that
    /// would hammer the LLM or render the sprite useless.
    pub fn clamped(&self) -> Self {
        let mut c = self.clone();
        c.idle_edit_secs = c.idle_edit_secs.clamp(3, 60);
        c.min_change_chars = c.min_change_chars.clamp(20, 2000);
        // Upper bound kept below the throttle TtlCache TTL (2h) so a configured
        // cooldown can never outlive its own throttle entry (idle eviction would
        // otherwise let an observation through before the cooldown elapsed).
        c.cooldown_secs = c.cooldown_secs.clamp(10, 3600);
        c.max_per_session_per_hour = c.max_per_session_per_hour.clamp(1, 60);
        c.periodic_secs = c.periodic_secs.clamp(15, 600);
        c.paste_min_chars = c.paste_min_chars.clamp(40, 4000);
        c.max_tokens = c.max_tokens.clamp(64, 1200);
        c.timeout_secs = c.timeout_secs.clamp(5, 60);
        c
    }
}
