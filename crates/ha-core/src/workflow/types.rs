use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunState {
    Draft,
    AwaitingApproval,
    Running,
    AwaitingUser,
    Paused,
    Recovering,
    Completed,
    Failed,
    Cancelled,
    Blocked,
}

impl WorkflowRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowRunState::Draft => "draft",
            WorkflowRunState::AwaitingApproval => "awaiting_approval",
            WorkflowRunState::Running => "running",
            WorkflowRunState::AwaitingUser => "awaiting_user",
            WorkflowRunState::Paused => "paused",
            WorkflowRunState::Recovering => "recovering",
            WorkflowRunState::Completed => "completed",
            WorkflowRunState::Failed => "failed",
            WorkflowRunState::Cancelled => "cancelled",
            WorkflowRunState::Blocked => "blocked",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "awaiting_approval" => Some(Self::AwaitingApproval),
            "running" => Some(Self::Running),
            "awaiting_user" => Some(Self::AwaitingUser),
            "paused" => Some(Self::Paused),
            "recovering" => Some(Self::Recovering),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Blocked
        )
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Running | Self::AwaitingUser | Self::Paused | Self::Recovering
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match (self, next) {
            (Self::Draft, Self::AwaitingApproval | Self::Running | Self::Cancelled) => true,
            (Self::AwaitingApproval, Self::Running | Self::Cancelled | Self::Blocked) => true,
            (
                Self::Running,
                Self::AwaitingUser
                | Self::Paused
                | Self::Recovering
                | Self::Completed
                | Self::Failed
                | Self::Cancelled
                | Self::Blocked,
            ) => true,
            (Self::AwaitingUser, Self::Running | Self::Cancelled | Self::Blocked) => true,
            (Self::Paused, Self::Running | Self::Cancelled | Self::Blocked) => true,
            (
                Self::Recovering,
                Self::Running | Self::Completed | Self::Failed | Self::Cancelled | Self::Blocked,
            ) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOpState {
    Pending,
    Started,
    Completed,
    Failed,
}

impl WorkflowOpState {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowOpState::Pending => "pending",
            WorkflowOpState::Started => "started",
            WorkflowOpState::Completed => "completed",
            WorkflowOpState::Failed => "failed",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "started" => Some(Self::Started),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEffectClass {
    Pure,
    Idempotent,
    NonIdempotent,
}

impl WorkflowEffectClass {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowEffectClass::Pure => "pure",
            WorkflowEffectClass::Idempotent => "idempotent",
            WorkflowEffectClass::NonIdempotent => "non_idempotent",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "pure" => Some(Self::Pure),
            "idempotent" => Some(Self::Idempotent),
            "non_idempotent" => Some(Self::NonIdempotent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub state: WorkflowRunState,
    pub loop_mode: String,
    pub script_hash: String,
    pub script_source: String,
    pub budget: Value,
    pub cursor_seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowOp {
    pub id: String,
    pub run_id: String,
    pub op_key: String,
    pub op_type: String,
    pub effect_class: WorkflowEffectClass,
    pub input_hash: String,
    pub input: Value,
    pub state: WorkflowOpState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_handle: Option<String>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEvent {
    pub id: i64,
    pub run_id: String,
    pub seq: i64,
    pub event_type: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowRunInput {
    pub session_id: String,
    pub kind: String,
    #[serde(default = "default_loop_mode")]
    pub loop_mode: String,
    pub script_source: String,
    #[serde(default)]
    pub budget: Value,
}

fn default_loop_mode() -> String {
    "guarded".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertWorkflowOpInput {
    pub run_id: String,
    pub op_key: String,
    pub op_type: String,
    pub effect_class: WorkflowEffectClass,
    pub input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_handle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSnapshot {
    pub run: WorkflowRun,
    pub ops: Vec<WorkflowOp>,
    pub events: Vec<WorkflowEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartedOpRecoveryAction {
    RerunPure,
    RecheckIdempotent,
    BlockNonIdempotent,
}
