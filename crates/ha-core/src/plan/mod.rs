mod constants;
mod file_io;
mod gates;
mod git;
mod index;
mod store;
mod subagent;
#[cfg(test)]
mod tests;
mod transition;
mod types;

// ── Re-exports ──────────────────────────────────────────────────

// Types
pub use types::{PlanAgentConfig, PlanMeta, PlanModeState, PlanVersionInfo};

// Constants
pub use constants::is_plan_mode_path_allowed;
pub use constants::{
    PLAN_COMPLETED_SYSTEM_PROMPT, PLAN_EXECUTING_SYSTEM_PROMPT_PREFIX, PLAN_MODE_ASK_TOOLS,
    PLAN_MODE_DENIED_TOOLS, PLAN_MODE_PATH_AWARE_TOOLS, PLAN_MODE_SYSTEM_PROMPT,
};

// Quality gates
pub use gates::{
    check_plan_quality, check_workflow_script_draft, GateIssue, GateReport, GateSeverity,
    ScriptGateOptions,
};

// Store
pub use store::store;
pub use store::{
    get_plan_meta, get_plan_state, restore_from_db, set_plan_state,
    should_create_execution_checkpoint,
};

// File I/O
pub use file_io::migrate_flat_plans_to_subdirs;
pub use file_io::{
    delete_plan_file, extract_plan_title, find_plan_file, list_plan_versions, load_plan_file,
    load_plan_version, save_plan_file,
};

// Cross-session index (read-only)
pub use index::{
    list_all_plans, resolve_plan_mention, PlanIndexEntry, PlanIndexFilter, PlanMentionResolution,
};

// Git
pub use git::{
    cleanup_checkpoint, create_checkpoint_for_session, create_git_checkpoint, get_checkpoint_ref,
    rollback_to_checkpoint,
};

// Subagent
pub use subagent::{
    get_active_plan_run_id, get_plan_owner_session_id, register_plan_subagent, spawn_plan_subagent,
    try_unregister_plan_subagent_sync, unregister_plan_subagent,
};

// Transition (centralized side-effect helper)
pub use transition::{maybe_complete_plan, transition_state, TransitionOutcome};
