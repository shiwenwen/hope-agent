use serde::{Deserialize, Serialize};

// ── Team Status ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamStatus {
    Active,
    Paused,
    Dissolved,
}

impl TeamStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Dissolved => "dissolved",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "paused" => Self::Paused,
            "dissolved" => Self::Dissolved,
            _ => Self::Active,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }
}

// ── Member Role ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Lead,
    Worker,
    Reviewer,
}

impl MemberRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "lead" => Self::Lead,
            "reviewer" => Self::Reviewer,
            _ => Self::Worker,
        }
    }
}

// ── Member Status ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    Idle,
    Working,
    Paused,
    Completed,
    Error,
    Killed,
}

impl MemberStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Killed => "killed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "working" => Self::Working,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "error" => Self::Error,
            "killed" => Self::Killed,
            _ => Self::Idle,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Error | Self::Killed)
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Idle | Self::Working)
    }
}

// ── Message Type ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMessageType {
    Chat,
    TaskUpdate,
    Handoff,
    System,
}

impl TeamMessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::TaskUpdate => "task_update",
            Self::Handoff => "handoff",
            Self::System => "system",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "task_update" => Self::TaskUpdate,
            "handoff" => Self::Handoff,
            "system" => Self::System,
            _ => Self::Chat,
        }
    }
}

// ── Team Config ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamConfig {
    #[serde(default = "default_max_members")]
    pub max_members: u32,
    #[serde(default)]
    pub auto_dissolve_on_complete: bool,
    #[serde(default)]
    pub shared_context: Option<String>,
}

fn default_max_members() -> u32 {
    super::DEFAULT_MAX_MEMBERS
}

impl Default for TeamConfig {
    fn default() -> Self {
        Self {
            max_members: default_max_members(),
            auto_dissolve_on_complete: false,
            shared_context: None,
        }
    }
}

// ── Team ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    pub lead_session_id: String,
    pub lead_agent_id: String,
    pub status: TeamStatus,
    pub created_at: String,
    pub updated_at: String,
    pub template_id: Option<String>,
    pub config: TeamConfig,
}

// ── Team Member ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    pub member_id: String,
    pub team_id: String,
    pub name: String,
    pub agent_id: String,
    pub role: MemberRole,
    pub status: MemberStatus,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub color: String,
    pub current_task_id: Option<i64>,
    pub model_override: Option<String>,
    pub joined_at: String,
    pub last_active_at: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

// ── Team Message ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMessage {
    pub message_id: String,
    pub team_id: String,
    pub from_member_id: String,
    pub to_member_id: Option<String>,
    pub content: String,
    pub message_type: TeamMessageType,
    pub timestamp: String,
}

// ── Team Task ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamTask {
    pub id: i64,
    pub team_id: String,
    pub content: String,
    pub status: String,
    pub owner_member_id: Option<String>,
    pub priority: u32,
    pub blocked_by: Vec<i64>,
    pub blocks: Vec<i64>,
    pub column_name: String,
    pub created_at: String,
    pub updated_at: String,
}

// ── Team Template ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamTemplate {
    pub template_id: String,
    pub name: String,
    pub description: String,
    pub members: Vec<TeamTemplateMember>,
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamTemplateMember {
    pub name: String,
    pub role: MemberRole,
    pub agent_id: String,
    pub color: String,
    pub description: String,
}

// ── Create Team Request (used by coordinator) ───────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamMemberSpec {
    pub name: String,
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
    #[serde(default)]
    pub role: Option<String>,
    pub task: String,
    #[serde(default)]
    pub model: Option<String>,
}

fn default_agent_id() -> String {
    "default".to_string()
}
