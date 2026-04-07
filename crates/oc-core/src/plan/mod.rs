mod constants;
mod file_io;
mod git;
mod parser;
mod questions;
mod store;
mod subagent;
#[cfg(test)]
mod tests;
mod types;

// ── Re-exports ──────────────────────────────────────────────────

// Types
pub use types::{
    PlanAgentConfig, PlanMeta, PlanModeState, PlanQuestion, PlanQuestionAnswer,
    PlanQuestionGroup, PlanQuestionOption, PlanStep, PlanStepStatus, PlanVersionInfo,
};

// Constants
pub use constants::{
    BUILD_AGENT_EXTRA_TOOLS, PLAN_COMPLETED_SYSTEM_PROMPT, PLAN_EXECUTING_SYSTEM_PROMPT_PREFIX,
    PLAN_MODE_ASK_TOOLS, PLAN_MODE_DENIED_TOOLS, PLAN_MODE_PATH_AWARE_TOOLS,
    PLAN_MODE_SYSTEM_PROMPT,
};
pub use constants::is_plan_mode_path_allowed;

// Store
pub use store::{
    get_plan_meta, get_plan_state, restore_from_db, set_plan_state, update_plan_steps,
    update_step_status,
};
pub use store::store;

// File I/O
pub use file_io::{
    delete_plan_file, list_plan_versions, load_plan_file, load_plan_version, save_plan_file,
    save_result_file,
};

// Parser
pub use parser::parse_plan_steps;

// Git
pub use git::{
    cleanup_checkpoint, create_checkpoint_for_session, create_git_checkpoint, get_checkpoint_ref,
    rollback_to_checkpoint,
};

// Questions
pub use questions::{
    cancel_pending_plan_question, register_plan_question, submit_plan_question_response,
};

// Subagent
pub use subagent::{
    get_active_plan_run_id, get_plan_owner_session_id, register_plan_subagent,
    spawn_plan_subagent, try_unregister_plan_subagent_sync, unregister_plan_subagent,
};
