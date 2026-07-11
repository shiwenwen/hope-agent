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
        matches!(
            (self, next),
            (
                Self::Draft,
                Self::AwaitingApproval | Self::Running | Self::Cancelled | Self::Blocked,
            ) | (
                Self::AwaitingApproval,
                Self::Running | Self::Cancelled | Self::Blocked
            ) | (
                Self::Running,
                Self::AwaitingUser
                    | Self::Paused
                    | Self::Recovering
                    | Self::Completed
                    | Self::Failed
                    | Self::Cancelled
                    | Self::Blocked,
            ) | (
                Self::AwaitingUser,
                Self::Running | Self::Cancelled | Self::Blocked
            ) | (
                Self::Paused,
                Self::Running | Self::Cancelled | Self::Blocked
            ) | (
                Self::Recovering,
                Self::Running | Self::Completed | Self::Failed | Self::Cancelled | Self::Blocked,
            )
        )
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
    pub execution_mode: String,
    pub script_hash: String,
    pub script_source: String,
    pub budget: Value,
    pub cursor_seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_criterion_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_criterion_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_criterion_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
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
pub struct PendingWorkflowMilestoneInjection {
    pub run_id: String,
    pub source_event_type: String,
    pub source_event_seq: i64,
    pub requested_event_seq: i64,
    pub requested_at: String,
    pub source_event: WorkflowEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowRunInput {
    pub session_id: String,
    pub kind: String,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    pub script_source: String,
    #[serde(default)]
    pub budget: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_criterion_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunControlInput {
    #[serde(default = "default_workflow_api_version")]
    pub api_version: i64,
    #[serde(default)]
    pub meta: Value,
    #[serde(default)]
    pub args: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_from_run_id: Option<String>,
}

impl Default for WorkflowRunControlInput {
    fn default() -> Self {
        Self {
            api_version: default_workflow_api_version(),
            meta: json_object(),
            args: json_object(),
            resume_from_run_id: None,
        }
    }
}

fn default_workflow_api_version() -> i64 {
    4
}

fn json_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunControl {
    pub run_id: String,
    pub api_version: i64,
    pub meta: Value,
    pub meta_hash: String,
    pub args: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_from_run_id: Option<String>,
    pub args_hash: String,
    pub created_at: String,
}

fn default_execution_mode() -> String {
    "guarded".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedWorkflowTemplateScope {
    User,
    Project,
}

impl SavedWorkflowTemplateScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "project" => Some(Self::Project),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWorkflowTemplate {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub scope: SavedWorkflowTemplateScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub kind: String,
    pub execution_mode: String,
    pub script_hash: String,
    pub script_source: String,
    pub budget: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSavedWorkflowTemplatesInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub include_disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWorkflowTemplateInput {
    pub source_run_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub scope: SavedWorkflowTemplateScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub explicit_save_consent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowRunFromTemplateInput {
    pub session_id: String,
    pub template_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_criterion_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
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
    pub agent_usage: WorkflowAgentUsageSnapshot,
    pub usage: WorkflowRunUsageSnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAgentUsageSnapshot {
    pub spawned_agents: i64,
    pub completed_agents: i64,
    pub running_agents: i64,
    pub failed_agents: i64,
    pub terminal_agents: i64,
    pub consumed_results: i64,
    pub pending_results: i64,
    pub suppressed_results: i64,
    pub attributed_agents: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub attribution: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunUsageSnapshot {
    pub parent_events: i64,
    pub parent_input_tokens: i64,
    pub parent_output_tokens: i64,
    pub parent_cache_creation_input_tokens: i64,
    pub parent_cache_read_input_tokens: i64,
    pub parent_total_tokens: i64,
    pub parent_injection_turns: i64,
    pub parent_injection_messages: i64,
    pub parent_injection_input_tokens: i64,
    pub parent_injection_output_tokens: i64,
    pub parent_injection_total_tokens: i64,
    pub parent_injection_provider_events: i64,
    pub parent_injection_provider_input_tokens: i64,
    pub parent_injection_provider_output_tokens: i64,
    pub parent_injection_provider_cache_creation_input_tokens: i64,
    pub parent_injection_provider_cache_read_input_tokens: i64,
    pub parent_injection_provider_total_tokens: i64,
    pub parent_injection_attribution: String,
    pub agent_input_tokens: i64,
    pub agent_output_tokens: i64,
    pub agent_total_tokens: i64,
    pub total_tokens: i64,
    pub attribution: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowWatchdogFinding {
    pub run_id: String,
    pub session_id: String,
    pub severity: String,
    pub code: String,
    pub message: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_event_seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartedOpRecoveryAction {
    RerunPure,
    RecheckIdempotent,
    AttachChildHandle(String),
    BlockNonIdempotent,
}
