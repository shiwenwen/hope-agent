pub mod types;
pub mod db;
pub mod coordinator;
pub mod messaging;
pub mod tasks;
pub mod templates;
pub mod events;
pub mod cleanup;

pub use types::*;

// ── Constants ───────────────────────────────────────────────────

/// Maximum members per team (configurable per-team via TeamConfig)
pub const DEFAULT_MAX_MEMBERS: u32 = 8;

/// Maximum active teams per agent
pub const MAX_ACTIVE_TEAMS: u32 = 3;

/// Color palette for team members (assigned round-robin)
pub const MEMBER_COLORS: &[&str] = &[
    "#3B82F6", // blue
    "#10B981", // emerald
    "#F59E0B", // amber
    "#EF4444", // red
    "#8B5CF6", // violet
    "#EC4899", // pink
    "#06B6D4", // cyan
    "#F97316", // orange
];

/// Pick a color for the nth member.
pub fn pick_member_color(index: usize) -> &'static str {
    MEMBER_COLORS[index % MEMBER_COLORS.len()]
}
