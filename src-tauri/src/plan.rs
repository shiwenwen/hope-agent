use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::RwLock;

// ── Plan Mode State ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanModeState {
    Off,
    Planning,
    Review,
    Executing,
    Paused,
    Completed,
}

impl Default for PlanModeState {
    fn default() -> Self {
        Self::Off
    }
}

impl PlanModeState {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Off => "off",
            Self::Planning => "planning",
            Self::Review => "review",
            Self::Executing => "executing",
            Self::Paused => "paused",
            Self::Completed => "completed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "planning" => Self::Planning,
            "review" => Self::Review,
            "executing" => Self::Executing,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            _ => Self::Off,
        }
    }
}

// ── Plan Step ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
    Failed,
}

impl PlanStepStatus {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Skipped | Self::Failed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub index: usize,
    pub phase: String,
    pub title: String,
    pub description: String,
    pub status: PlanStepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

// ── Plan Metadata ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanMeta {
    pub session_id: String,
    pub title: Option<String>,
    pub file_path: String,
    pub state: PlanModeState,
    pub steps: Vec<PlanStep>,
    pub created_at: String,
    pub updated_at: String,
    /// Step index where execution was paused (for Paused state)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_at_step: Option<usize>,
    /// Plan version counter (incremented on each save/edit)
    #[serde(default = "default_version")]
    pub version: u32,
    /// Git checkpoint reference (branch or stash) created before execution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_ref: Option<String>,
}

fn default_version() -> u32 {
    1
}

impl PlanMeta {
    #[allow(dead_code)]
    pub fn completed_count(&self) -> usize {
        self.steps.iter().filter(|s| s.status.is_terminal()).count()
    }

    pub fn all_terminal(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|s| s.status.is_terminal())
    }
}

// ── Plan Question (Interactive Planning) ────────────────────────

/// A single question option for the user to choose from
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanQuestionOption {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this option is recommended/suggested as the default choice.
    #[serde(default)]
    pub recommended: bool,
}

/// A structured question sent by LLM to the user during planning
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanQuestion {
    pub question_id: String,
    pub text: String,
    pub options: Vec<PlanQuestionOption>,
    #[serde(default = "default_true")]
    pub allow_custom: bool,
    #[serde(default)]
    pub multi_select: bool,
    /// Optional question template/category (e.g., "scope", "tech_choice", "priority")
    /// Used to render category-specific UI styling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

fn default_true() -> bool {
    true
}

/// A group of questions sent together
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanQuestionGroup {
    pub request_id: String,
    pub session_id: String,
    pub questions: Vec<PlanQuestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// User's answer to a single question
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanQuestionAnswer {
    pub question_id: String,
    pub selected: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_input: Option<String>,
}

// ── Pending Plan Questions Registry (oneshot pattern) ────────────

static PENDING_PLAN_QUESTIONS: OnceLock<
    TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<PlanQuestionAnswer>>>>,
> = OnceLock::new();

fn get_pending_questions(
) -> &'static TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<PlanQuestionAnswer>>>> {
    PENDING_PLAN_QUESTIONS.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Register a pending question and return the receiver.
pub async fn register_plan_question(
    request_id: String,
    sender: tokio::sync::oneshot::Sender<Vec<PlanQuestionAnswer>>,
) {
    let mut pending = get_pending_questions().lock().await;
    pending.insert(request_id, sender);
}

/// Submit answers from the frontend (called by Tauri command).
pub async fn submit_plan_question_response(
    request_id: &str,
    answers: Vec<PlanQuestionAnswer>,
) -> Result<()> {
    let mut pending = get_pending_questions().lock().await;
    if let Some(sender) = pending.remove(request_id) {
        let _ = sender.send(answers);
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "No pending plan question request: {}",
            request_id
        ))
    }
}

/// Cancel a pending question (e.g., on plan exit).
pub async fn cancel_pending_plan_question(request_id: &str) {
    let mut pending = get_pending_questions().lock().await;
    pending.remove(request_id);
}

// ── Plan Agent / Build Agent Configuration ─────────────────────

/// Declarative configuration for the Plan Agent (Planning/Review states).
/// Uses an **allow-list** approach: only listed tools are available.
pub struct PlanAgentConfig {
    /// Tool allow-list: only these tools are available to the Plan Agent
    pub allowed_tools: Vec<String>,
    /// Path restrictions for write/edit (only .md in plans/ directory)
    pub plan_mode_allow_paths: Vec<String>,
    /// Tools that require user approval (e.g., exec)
    pub ask_tools: Vec<String>,
}

impl PlanAgentConfig {
    pub fn default_config() -> Self {
        Self {
            allowed_tools: vec![
                // Read-only exploration tools
                "read",
                "ls",
                "grep",
                "find",
                "glob",
                "web_search",
                "web_fetch",
                // Restricted execution (requires approval)
                "exec",
                // Plan-specific tools
                "plan_question",
                "submit_plan",
                // Path-restricted write tools (only plans/ directory)
                "write",
                "edit",
                // Memory and delegation
                "recall_memory",
                "memory_get",
                "subagent",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            plan_mode_allow_paths: vec!["plans".into()],
            ask_tools: vec!["exec".into()],
        }
    }
}

/// Extra tools injected for the Build Agent (Executing/Paused states).
pub const BUILD_AGENT_EXTRA_TOOLS: &[&str] = &["update_plan_step", "amend_plan"];

/// Tools denied in Plan Mode — kept for sub-agent inheritance compatibility.
/// Derived from PlanAgentConfig: tools NOT in the allow-list.
pub const PLAN_MODE_DENIED_TOOLS: &[&str] = &["write", "edit", "apply_patch", "canvas"];

#[allow(dead_code)]
pub const PLAN_MODE_ASK_TOOLS: &[&str] = &["exec"];

/// Tools that support path-based allow in Plan Mode.
/// During Planning, these tools are normally denied, but if the file path targets
#[allow(dead_code)]
pub const PLAN_MODE_PATH_AWARE_TOOLS: &[&str] = &["write", "edit"];

/// Check if a file path is allowed during Plan Mode (targets a plan file).
pub fn is_plan_mode_path_allowed(file_path: &str) -> bool {
    let path = std::path::Path::new(file_path);
    // Allow writes to any .md file under ~/.opencomputer/plans/
    if let Some(ext) = path.extension() {
        if ext != "md" {
            return false;
        }
    } else {
        return false;
    }
    // Check if any ancestor directory is named "plans" under an ".opencomputer" dir
    let path_str = file_path.replace('\\', "/");
    if path_str.contains(".opencomputer/plans/") || path_str.contains(".opencomputer\\plans\\") {
        return true;
    }
    // Also allow if the file is directly in the plans_dir
    if let Ok(plans) = plans_dir() {
        let plans_str = plans.to_string_lossy().replace('\\', "/");
        if path_str.starts_with(&plans_str) {
            return true;
        }
    }
    false
}

// ── Plan Sub-Agent Session Registry ─────────────────────────────
// Maps child_session_id → parent info, so plan tools (plan_question, submit_plan)
// can route events to the parent session instead of the sub-agent session.

struct PlanSubagentInfo {
    parent_session_id: String,
    run_id: String,
}

static PLAN_SUBAGENT_SESSIONS: OnceLock<Arc<RwLock<HashMap<String, PlanSubagentInfo>>>> =
    OnceLock::new();

fn plan_subagent_store() -> &'static Arc<RwLock<HashMap<String, PlanSubagentInfo>>> {
    PLAN_SUBAGENT_SESSIONS.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

/// Register a plan sub-agent session mapping.
pub async fn register_plan_subagent(child_sid: &str, parent_sid: &str, run_id: &str) {
    let mut map = plan_subagent_store().write().await;
    map.insert(
        child_sid.to_string(),
        PlanSubagentInfo {
            parent_session_id: parent_sid.to_string(),
            run_id: run_id.to_string(),
        },
    );
    app_info!(
        "plan",
        "subagent",
        "Registered plan sub-agent: child={} parent={} run={}",
        child_sid,
        parent_sid,
        run_id
    );
}

/// Unregister a plan sub-agent session mapping.
#[allow(dead_code)]
pub async fn unregister_plan_subagent(child_sid: &str) {
    let mut map = plan_subagent_store().write().await;
    if map.remove(child_sid).is_some() {
        app_info!(
            "plan",
            "subagent",
            "Unregistered plan sub-agent: child={}",
            child_sid
        );
    }
}

/// Synchronous version for use in non-async contexts (e.g., spawn completion callback).
/// Spawns a blocking task to do the cleanup.
pub fn try_unregister_plan_subagent_sync(child_sid: &str) {
    let sid = child_sid.to_string();
    let store = plan_subagent_store().clone();
    tokio::spawn(async move {
        let mut map = store.write().await;
        if map.remove(&sid).is_some() {
            app_info!(
                "plan",
                "subagent",
                "Unregistered plan sub-agent (sync): child={}",
                sid
            );
        }
    });
}

/// If this session_id belongs to a plan sub-agent, return the parent session_id.
pub async fn get_plan_owner_session_id(session_id: &str) -> Option<String> {
    let map = plan_subagent_store().read().await;
    map.get(session_id)
        .map(|info| info.parent_session_id.clone())
}

/// Get the active plan sub-agent run_id for a parent session, if any.
pub async fn get_active_plan_run_id(parent_session_id: &str) -> Option<String> {
    let map = plan_subagent_store().read().await;
    map.values()
        .find(|info| info.parent_session_id == parent_session_id)
        .map(|info| info.run_id.clone())
}

/// Spawn a dedicated plan sub-agent for the Planning phase.
/// Returns the run_id. The sub-agent runs with PlanAgent mode and PLAN_MODE_SYSTEM_PROMPT.
pub async fn spawn_plan_subagent(
    parent_session_id: &str,
    parent_agent_id: &str,
    user_message: &str,
    recent_context_summary: &str,
    session_db: std::sync::Arc<crate::session::SessionDB>,
    cancel_registry: std::sync::Arc<crate::subagent::SubagentCancelRegistry>,
) -> Result<String> {
    let config = PlanAgentConfig::default_config();

    let task = if recent_context_summary.is_empty() {
        user_message.to_string()
    } else {
        format!(
            "## User Request\n{}\n\n## Conversation Context\n{}",
            user_message, recent_context_summary
        )
    };

    let params = crate::subagent::SpawnParams {
        task,
        agent_id: parent_agent_id.to_string(),
        parent_session_id: parent_session_id.to_string(),
        parent_agent_id: parent_agent_id.to_string(),
        depth: 1,
        timeout_secs: Some(3600), // 1 hour — plan_question can wait 10 min each
        model_override: None,
        label: Some("Plan Creation".to_string()),
        attachments: Vec::new(),
        plan_agent_mode: Some(crate::agent::PlanAgentMode::PlanAgent {
            allowed_tools: config.allowed_tools,
            ask_tools: config.ask_tools,
        }),
        plan_mode_allow_paths: config.plan_mode_allow_paths,
        skip_parent_injection: true,
        extra_system_context: Some(format!(
            "{}\n\n{}",
            PLAN_MODE_SYSTEM_PROMPT, PLAN_SUBAGENT_CONTEXT_NOTICE
        )),
    };

    let run_id =
        crate::subagent::spawn_subagent(params, session_db.clone(), cancel_registry).await?;

    // Get child_session_id from the run record
    if let Ok(Some(run)) = session_db.get_subagent_run(&run_id) {
        register_plan_subagent(&run.child_session_id, parent_session_id, &run_id).await;
    }

    app_info!(
        "plan",
        "subagent",
        "Spawned plan sub-agent: run_id={} parent_session={}",
        run_id,
        parent_session_id
    );

    // Emit status event to frontend
    if let Some(app_handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = app_handle.emit(
            "plan_subagent_status",
            serde_json::json!({
                "sessionId": parent_session_id,
                "status": "running",
                "runId": run_id,
            }),
        );
    }

    Ok(run_id)
}

// ── System Prompt ───────────────────────────────────────────────

/// Extra context appended to PLAN_MODE_SYSTEM_PROMPT when running as a sub-agent.
/// Reminds the LLM that the executing agent has NO exploration history.
const PLAN_SUBAGENT_CONTEXT_NOTICE: &str = "\
## Sub-Agent Context Notice

You are running as a **plan creation sub-agent**. The executing agent will NOT have \
access to your exploration history — only the plan you submit via `submit_plan`.

Your plan must be **self-contained**:
- Include code snippets for ALL new structs, types, and key functions
- Quote relevant existing code that the executor needs to understand
- Specify exact file paths, line ranges, and function signatures
- Document all dependencies and imports needed
- The plan IS the only context — make it complete enough to execute without re-exploration";

pub const PLAN_MODE_SYSTEM_PROMPT: &str = "\
# Plan Mode Active

You are in **Plan Mode**. Create a comprehensive, high-quality implementation plan through structured exploration and interactive Q&A.

## Restrictions
- You **CANNOT** modify project source files (apply_patch, canvas tools are disabled)
- You **CAN** use `write` and `edit` tools **only on plan files** (under `~/.opencomputer/plans/`)
- You **CAN** read files, search code, browse the web, and analyze the codebase
- Shell commands (exec) require user approval before execution

## 5-Phase Planning Workflow

### Phase 1: Deep Exploration
**Goal**: Thoroughly understand the codebase before making any decisions.
- Use the `subagent` tool to spawn **parallel exploration tasks** for faster analysis
  - Example: spawn one subagent to read API layer, another for database schema, another for frontend components
  - You can run up to 3 exploration subagents in parallel
- Read relevant source files, search for patterns, understand dependencies
- Map the affected modules, interfaces, and data flow
- Identify potential risks, edge cases, and constraints

### Phase 2: Requirements Clarification
**Goal**: Ensure complete understanding of user intent.
- Use the `plan_question` tool to ask structured questions with suggested options
  - Group related questions together (send multiple in one call)
  - Provide 2-5 suggested options per question with clear labels and descriptions
  - Set `allow_custom=true` when the user might have a different idea
  - Use `multi_select=true` when multiple options can apply
  - Mark the best option with `recommended=true` to highlight it (renders with a ★ badge)
  - Use `template` field for category-specific UI: `scope`, `tech_choice`, `priority`
- Ask about: scope, technical approach, priority, testing strategy, edge cases
- After receiving answers, you may ask follow-up questions if needed

### Phase 3: Design & Architecture
**Goal**: Design the solution approach based on exploration findings and user requirements.
- Consider alternative approaches and their trade-offs
- Identify which files need to be created, modified, or deleted
- Consider backward compatibility, performance impact, and error handling
- If needed, use subagent to validate assumptions (e.g., check if a library supports a feature)

### Phase 4: Plan Composition
**Goal**: Write a detailed, actionable implementation plan.
- Use the `submit_plan` tool to submit the final plan
- Plan must follow the format below

### Phase 5: Review & Refinement
**Goal**: Let the user review and refine the plan before execution.
- After submitting, the plan enters Review state
- User can approve, request changes, or exit
- User may provide inline comments on specific plan sections (wrapped in `<plan-inline-comment>` tags). \
When you receive an inline comment, revise the referenced `<selected-text>` section based on the \
`<revision-request>`, then resubmit the full updated plan via `submit_plan`

## Tools
- `plan_question`: Send structured questions to the user with suggested options (renders as interactive UI cards)
- `submit_plan`: Submit the final plan (title + markdown content with phases and checklists)
- `subagent`: Spawn parallel exploration tasks for faster codebase analysis
- All read-only tools (read, search, glob, web_search, web_fetch, etc.)

## Plan Format (for submit_plan content)

Your plan must be **implementation-ready** — an executor should be able to follow it \
without re-reading the codebase. Structure by files, not by abstract phases.

### Required Sections:

**Context** (2-3 sentences only)
What problem this solves and the chosen approach. Do NOT restate the user's request verbatim.

**Steps** (the core of the plan — organize by file or logical unit)

For each step:
- Step title includes file path: `### Step N: <verb> — <file_path>`
- What to create or modify, with enough detail to execute
- **Code blocks** for new struct/type definitions, function signatures, key logic
- **Tables** for mappings when applicable (field → type → source)
- Existing utilities to reuse (with `file_path:line` references)
- Wire-up: where to register, import, or connect this change
- Use `- [ ]` sub-tasks for trackable items within a step

**Verification** (concrete test commands or manual steps)

### Example:

```markdown
## Context
添加 URL 预览功能：消息和输入框中的 URL 自动抓取 OpenGraph 元数据并展示预览卡片。

### Step 1: 后端 — `src-tauri/src/url_preview.rs`

新建模块，定义结构体并实现轻量抓取：

‍```rust
pub struct UrlPreviewMeta {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
}
‍```

- [ ] 用 regex 提取 `<meta property=\"og:*\">` / `<title>`
- [ ] 复用 `web_fetch.rs` 的 `check_ssrf_safe()` (line 45)
- [ ] 独立内存缓存（100 条，TTL 5 分钟）

### Step 2: 后端 — `src-tauri/src/commands/url_preview.rs`

- [ ] 新增 `fetch_url_preview(url: String) -> Result<UrlPreviewMeta, String>`
- [ ] 注册到 `lib.rs` invoke_handler

## Verification
cargo check && npx tsc --noEmit
```

## Guidelines
- Structure by **files**, not abstract phases — each step title includes a file path
- Use **code blocks** for struct definitions, type interfaces, function signatures
- Reference existing code with `file_path:line` notation (e.g., `utils.rs:42`)
- List dependencies to reuse — avoid reinventing existing utilities
- Each step should be independently verifiable
- Include a **Verification** section with concrete test commands
- Do NOT add Background/Overview sections longer than 3 sentences
- Do NOT write steps that just say \"implement X\" without showing HOW
- Do NOT output the plan in chat messages — always use `submit_plan` tool";

/// System prompt injected when plan execution is completed.
pub const PLAN_COMPLETED_SYSTEM_PROMPT: &str = "\
# Plan Execution Completed

The plan has been fully executed. Here is a summary of the results:

## Your Tasks
1. **Summarize** what was accomplished in this plan
2. **Highlight** any steps that failed or were skipped, and explain why
3. **Suggest** follow-up actions if needed (e.g., testing, code review, further improvements)
4. **Answer** any questions the user has about the execution results

## Completed Plan

";

/// System prompt injected during plan execution phase.
pub const PLAN_EXECUTING_SYSTEM_PROMPT_PREFIX: &str = "\
# Executing Plan

You are executing an approved plan. Follow the steps below in order.
After completing each step, call the `update_plan_step` tool to mark your progress:
- `update_plan_step(step_index=N, status=\"in_progress\")` when starting a step
- `update_plan_step(step_index=N, status=\"completed\")` when done
- `update_plan_step(step_index=N, status=\"failed\")` if a step fails
- `update_plan_step(step_index=N, status=\"skipped\")` if skipping

A git checkpoint has been created before execution started. If execution fails, the user can rollback all changes.

If you discover the plan needs changes during execution, use the `amend_plan` tool:
- `amend_plan(action=\"insert\", title=\"New step\", after_index=N)` to add a step
- `amend_plan(action=\"delete\", step_index=N)` to remove a pending step
- `amend_plan(action=\"update\", step_index=N, title=\"Updated title\")` to modify a step

## Plan Content

";

// ── Global Per-Session Store ────────────────────────────────────

static PLAN_STORE: OnceLock<Arc<RwLock<HashMap<String, PlanMeta>>>> = OnceLock::new();

pub(crate) fn store() -> &'static Arc<RwLock<HashMap<String, PlanMeta>>> {
    PLAN_STORE.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

pub async fn get_plan_state(session_id: &str) -> PlanModeState {
    let map = store().read().await;
    map.get(session_id)
        .map(|m| m.state.clone())
        .unwrap_or(PlanModeState::Off)
}

pub async fn set_plan_state(session_id: &str, state: PlanModeState) {
    let mut map = store().write().await;
    if state == PlanModeState::Off {
        map.remove(session_id);
    } else if let Some(meta) = map.get_mut(session_id) {
        // Record paused_at_step when transitioning to Paused
        if state == PlanModeState::Paused {
            // Find the first in_progress step, or the first pending step
            let paused_at = meta
                .steps
                .iter()
                .position(|s| s.status == PlanStepStatus::InProgress)
                .or_else(|| {
                    meta.steps
                        .iter()
                        .position(|s| s.status == PlanStepStatus::Pending)
                });
            meta.paused_at_step = paused_at;
        } else if state == PlanModeState::Executing {
            // Clear paused_at_step when resuming
            meta.paused_at_step = None;
        }
        meta.state = state;
        meta.updated_at = chrono::Utc::now().to_rfc3339();
    } else {
        // Create a new PlanMeta entry
        let file_path = plan_file_path(session_id)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        map.insert(
            session_id.to_string(),
            PlanMeta {
                session_id: session_id.to_string(),
                title: None,
                file_path,
                state,
                steps: Vec::new(),
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                paused_at_step: None,
                version: 1,
                checkpoint_ref: None,
            },
        );
    }
}

pub async fn get_plan_meta(session_id: &str) -> Option<PlanMeta> {
    let map = store().read().await;
    map.get(session_id).cloned()
}

pub async fn update_plan_steps(session_id: &str, steps: Vec<PlanStep>) {
    let mut map = store().write().await;
    if let Some(meta) = map.get_mut(session_id) {
        meta.steps = steps.clone();
        meta.updated_at = chrono::Utc::now().to_rfc3339();
    }
    drop(map);
    // Persist to DB for crash recovery
    persist_steps_to_db(session_id, &steps);
}

pub async fn update_step_status(
    session_id: &str,
    step_index: usize,
    status: PlanStepStatus,
    duration_ms: Option<u64>,
) {
    let steps_snapshot;
    {
        let mut map = store().write().await;
        if let Some(meta) = map.get_mut(session_id) {
            if let Some(step) = meta.steps.get_mut(step_index) {
                step.status = status;
                if duration_ms.is_some() {
                    step.duration_ms = duration_ms;
                }
                meta.updated_at = chrono::Utc::now().to_rfc3339();
            }
            steps_snapshot = Some(meta.steps.clone());
        } else {
            steps_snapshot = None;
        }
    }
    // Persist step statuses to DB for crash recovery
    if let Some(steps) = steps_snapshot {
        persist_steps_to_db(session_id, &steps);
    }
}

/// Persist plan steps to DB as JSON (fire-and-forget, non-blocking).
fn persist_steps_to_db(session_id: &str, steps: &[PlanStep]) {
    if let Ok(json) = serde_json::to_string(steps) {
        if let Some(db) = crate::get_session_db() {
            let _ = db.save_plan_steps(session_id, &json);
        }
    }
}

/// Restore plan state from DB on session load.
/// First tries to load persisted step statuses from DB (crash-safe),
/// then falls back to re-parsing the plan markdown file.
pub async fn restore_from_db(session_id: &str, plan_mode_str: &str) {
    let state = PlanModeState::from_str(plan_mode_str);
    if state == PlanModeState::Off {
        return;
    }

    // Try loading persisted step statuses from DB first (crash recovery)
    let steps = if let Some(db) = crate::get_session_db() {
        if let Ok(Some(json)) = db.load_plan_steps(session_id) {
            serde_json::from_str::<Vec<PlanStep>>(&json).unwrap_or_default()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // Fallback: if DB had no steps, re-parse from plan file
    let steps = if steps.is_empty() {
        match load_plan_file(session_id) {
            Ok(Some(content)) => parse_plan_steps(&content),
            _ => Vec::new(),
        }
    } else {
        steps
    };

    let file_path = plan_file_path(session_id)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut map = store().write().await;
    map.insert(
        session_id.to_string(),
        PlanMeta {
            session_id: session_id.to_string(),
            title: None,
            file_path,
            state,
            steps,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            paused_at_step: None,
            version: 1,
            checkpoint_ref: None,
        },
    );
}

// ── Plan File I/O ───────────────────────────────────────────────
// Plans are stored in the workspace plan/ directory with readable names:
//   ~/.opencomputer/plans/plan-{short_id}-{timestamp}.md
//   ~/.opencomputer/plans/result-{short_id}-{timestamp}.md

fn plans_dir() -> Result<std::path::PathBuf> {
    crate::paths::plans_dir()
}

/// Build the plan file path for a session. Uses a mapping stored in PlanMeta.file_path.
/// If no existing path, generates a new one with readable name.
fn plan_file_path(session_id: &str) -> Result<std::path::PathBuf> {
    // Check if we already have a path in memory
    let store = PLAN_STORE.get_or_init(|| Arc::new(RwLock::new(HashMap::new())));
    if let Ok(map) = store.try_read() {
        if let Some(meta) = map.get(session_id) {
            if !meta.file_path.is_empty() {
                let p = std::path::PathBuf::from(&meta.file_path);
                if p.exists() {
                    return Ok(p);
                }
            }
        }
    }
    // Generate new path: plan-{short_id}-{date}.md
    let short_id = crate::truncate_utf8(session_id, 8);
    let date = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!("plan-{}-{}.md", short_id, date);
    Ok(plans_dir()?.join(filename))
}

/// Build the result file path for a session.
fn result_file_path(session_id: &str) -> Result<std::path::PathBuf> {
    let short_id = crate::truncate_utf8(session_id, 8);
    let date = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!("result-{}-{}.md", short_id, date);
    Ok(plans_dir()?.join(filename))
}

pub fn save_plan_file(session_id: &str, content: &str) -> Result<String> {
    let dir = plans_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = plan_file_path(session_id)?;

    // Version management: backup old version before overwriting
    if path.exists() {
        let current_version = {
            let store = PLAN_STORE.get_or_init(|| Arc::new(RwLock::new(HashMap::new())));
            if let Ok(map) = store.try_read() {
                map.get(session_id).map(|m| m.version).unwrap_or(1)
            } else {
                1
            }
        };
        // Copy current file to versioned backup: plan-xxx-v{N}.md
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let backup_name = format!("{}-v{}.md", stem, current_version);
        let backup_path = dir.join(&backup_name);
        if let Err(e) = std::fs::copy(&path, &backup_path) {
            app_warn!(
                "plan",
                "version",
                "Failed to backup plan version {}: {}",
                current_version,
                e
            );
        }
        // Increment version counter in memory
        tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                let store_ref = store();
                let mut map = store_ref.write().await;
                if let Some(meta) = map.get_mut(session_id) {
                    meta.version += 1;
                }
            });
        });
    }

    std::fs::write(&path, content)?;
    let path_str = path.to_string_lossy().to_string();
    // Update file_path in memory
    tokio::task::block_in_place(|| {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(async {
            let mut map = store().write().await;
            if let Some(meta) = map.get_mut(session_id) {
                meta.file_path = path_str.clone();
            }
        });
    });
    Ok(path_str)
}

pub fn load_plan_file(session_id: &str) -> Result<Option<String>> {
    let path = plan_file_path(session_id)?;
    if path.exists() {
        return Ok(Some(std::fs::read_to_string(path)?));
    }
    Ok(None)
}

#[allow(dead_code)]
pub fn delete_plan_file(session_id: &str) -> Result<()> {
    let path = plan_file_path(session_id)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Save execution result as a separate markdown file.
pub fn save_result_file(
    session_id: &str,
    plan_title: &str,
    steps: &[PlanStep],
    summary: &str,
) -> Result<String> {
    let dir = plans_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = result_file_path(session_id)?;

    let mut md = String::new();
    md.push_str(&format!("# 执行结果: {}\n\n", plan_title));
    md.push_str(&format!(
        "> 执行时间: {}\n\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));

    // Step results
    md.push_str("## 步骤执行情况\n\n");
    let mut current_phase = String::new();
    for step in steps {
        if step.phase != current_phase {
            current_phase = step.phase.clone();
            md.push_str(&format!("### {}\n\n", current_phase));
        }
        let icon = match step.status {
            PlanStepStatus::Completed => "✅",
            PlanStepStatus::Failed => "❌",
            PlanStepStatus::Skipped => "⏭️",
            PlanStepStatus::InProgress => "🔄",
            PlanStepStatus::Pending => "⭕",
        };
        let duration = step
            .duration_ms
            .map(|ms| format!(" ({}ms)", ms))
            .unwrap_or_default();
        md.push_str(&format!("- {} {}{}\n", icon, step.title, duration));
    }

    let completed = steps
        .iter()
        .filter(|s| s.status == PlanStepStatus::Completed)
        .count();
    let failed = steps
        .iter()
        .filter(|s| s.status == PlanStepStatus::Failed)
        .count();
    let skipped = steps
        .iter()
        .filter(|s| s.status == PlanStepStatus::Skipped)
        .count();
    md.push_str(&format!(
        "\n## 统计\n\n- 完成: {}\n- 失败: {}\n- 跳过: {}\n- 总计: {}\n",
        completed,
        failed,
        skipped,
        steps.len()
    ));

    if !summary.is_empty() {
        md.push_str(&format!("\n## 总结\n\n{}\n", summary));
    }

    std::fs::write(&path, &md)?;
    Ok(path.to_string_lossy().to_string())
}

/// List available versions of a plan (including the current and all backups).
pub fn list_plan_versions(session_id: &str) -> Result<Vec<PlanVersionInfo>> {
    let dir = plans_dir()?;
    let path = plan_file_path(session_id)?;
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut versions = Vec::new();

    // Current version
    if path.exists() {
        let meta = std::fs::metadata(&path)?;
        let modified = meta
            .modified()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_default();
        let current_version = {
            let store = PLAN_STORE.get_or_init(|| Arc::new(RwLock::new(HashMap::new())));
            if let Ok(map) = store.try_read() {
                map.get(session_id).map(|m| m.version).unwrap_or(1)
            } else {
                1
            }
        };
        versions.push(PlanVersionInfo {
            version: current_version,
            file_path: path.to_string_lossy().to_string(),
            modified_at: modified,
            is_current: true,
        });
    }

    // Backup versions
    if dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Match pattern: {stem}-v{N}.md
                if name.starts_with(&format!("{}-v", stem)) && name.ends_with(".md") {
                    let version_str = name
                        .trim_start_matches(&format!("{}-v", stem))
                        .trim_end_matches(".md");
                    if let Ok(v) = version_str.parse::<u32>() {
                        let meta = std::fs::metadata(entry.path()).ok();
                        let modified = meta
                            .and_then(|m| m.modified().ok())
                            .map(|t| {
                                let dt: chrono::DateTime<chrono::Local> = t.into();
                                dt.to_rfc3339()
                            })
                            .unwrap_or_default();
                        versions.push(PlanVersionInfo {
                            version: v,
                            file_path: entry.path().to_string_lossy().to_string(),
                            modified_at: modified,
                            is_current: false,
                        });
                    }
                }
            }
        }
    }

    // Sort by version descending (current first)
    versions.sort_by(|a, b| b.version.cmp(&a.version));
    Ok(versions)
}

/// Info about a plan version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanVersionInfo {
    pub version: u32,
    pub file_path: String,
    pub modified_at: String,
    pub is_current: bool,
}

/// Load content of a specific plan version.
pub fn load_plan_version(file_path: &str) -> Result<String> {
    Ok(std::fs::read_to_string(file_path)?)
}

// ── Markdown Checklist Parser ───────────────────────────────────

/// Parse a markdown plan into structured PlanStep items.
/// Expected format:
/// ```
/// ### Phase 1: Analysis
/// - [ ] Read config files
/// - [x] Analyze CSS variables
/// ### Phase 2: Implementation
/// - [ ] Add ThemeProvider
/// ```
pub fn parse_plan_steps(markdown: &str) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    let mut current_phase = String::new();
    let mut index = 0;

    for line in markdown.lines() {
        let trimmed = line.trim();

        // Match phase headers: "### Phase N: title" or "### title"
        if trimmed.starts_with("### ") {
            current_phase = trimmed.trim_start_matches("### ").to_string();
            continue;
        }

        // Match checklist items: "- [ ] text" or "- [x] text"
        if let Some(rest) = trimmed.strip_prefix("- [") {
            let (checked, text) = if let Some(t) = rest
                .strip_prefix("x] ")
                .or_else(|| rest.strip_prefix("X] "))
            {
                (true, t)
            } else if let Some(t) = rest.strip_prefix(" ] ") {
                (false, t)
            } else {
                continue;
            };

            let status = if checked {
                PlanStepStatus::Completed
            } else {
                PlanStepStatus::Pending
            };

            steps.push(PlanStep {
                index,
                phase: current_phase.clone(),
                title: text.to_string(),
                description: String::new(),
                status,
                duration_ms: None,
            });
            index += 1;
        }
    }

    steps
}

// ── Git Checkpoint ──────────────────────────────────────────────
// Creates a lightweight git checkpoint before plan execution starts,
// allowing rollback if execution fails.

/// Detect the git repository root directory by running `git rev-parse --show-toplevel`.
/// Returns None if not inside a git repository.
fn git_repo_root() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(path))
        }
    } else {
        None
    }
}

/// Create a git checkpoint (branch) at the current HEAD for the working directory.
/// Returns the checkpoint branch name on success, or None if not in a git repo.
pub fn create_git_checkpoint(session_id: &str) -> Option<String> {
    let short_id = crate::truncate_utf8(session_id, 8);
    let ts = chrono::Local::now().format("%Y%m%d%H%M%S");
    let branch_name = format!("opencomputer/checkpoint-{}-{}", short_id, ts);

    // Detect git repo root directory
    let git_root = git_repo_root()?;

    // Create a checkpoint branch at current HEAD (without switching to it)
    let result = std::process::Command::new("git")
        .current_dir(&git_root)
        .args(["branch", &branch_name, "HEAD"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match result {
        Ok(s) if s.success() => {
            app_info!(
                "plan",
                "checkpoint",
                "Created git checkpoint branch: {}",
                branch_name
            );
            Some(branch_name)
        }
        _ => {
            app_warn!(
                "plan",
                "checkpoint",
                "Failed to create git checkpoint branch"
            );
            None
        }
    }
}

/// Create a checkpoint and store it in the plan's metadata.
pub async fn create_checkpoint_for_session(session_id: &str) {
    if let Some(ref_name) = create_git_checkpoint(session_id) {
        let mut map = store().write().await;
        if let Some(meta) = map.get_mut(session_id) {
            meta.checkpoint_ref = Some(ref_name);
        }
    }
}

/// Get the checkpoint reference for a session.
pub async fn get_checkpoint_ref(session_id: &str) -> Option<String> {
    let map = store().read().await;
    map.get(session_id).and_then(|m| m.checkpoint_ref.clone())
}

/// Rollback to a git checkpoint by resetting the current branch to the checkpoint.
/// This performs a `git reset --hard <checkpoint_branch>` to undo all changes
/// made during plan execution.
pub fn rollback_to_checkpoint(checkpoint_ref: &str) -> Result<String> {
    let git_root = git_repo_root().ok_or_else(|| anyhow::anyhow!("Not inside a git repository"))?;

    // Verify the checkpoint branch exists
    let check = std::process::Command::new("git")
        .current_dir(&git_root)
        .args(["rev-parse", "--verify", checkpoint_ref])
        .output();
    match check {
        Ok(o) if o.status.success() => {}
        _ => {
            return Err(anyhow::anyhow!(
                "Checkpoint branch '{}' does not exist",
                checkpoint_ref
            ))
        }
    }

    // Get current HEAD for logging
    let head_before = std::process::Command::new("git")
        .current_dir(&git_root)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Reset to checkpoint
    let result = std::process::Command::new("git")
        .current_dir(&git_root)
        .args(["reset", "--hard", checkpoint_ref])
        .output()?;

    if result.status.success() {
        let msg = format!(
            "Rolled back from {} to checkpoint '{}'",
            head_before, checkpoint_ref
        );
        app_info!("plan", "checkpoint", "{}", msg);

        // Clean up: delete the checkpoint branch
        let _ = std::process::Command::new("git")
            .current_dir(&git_root)
            .args(["branch", "-D", checkpoint_ref])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        Ok(msg)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        Err(anyhow::anyhow!("Git reset failed: {}", stderr))
    }
}

/// Clean up a checkpoint branch (e.g., after successful execution).
pub fn cleanup_checkpoint(checkpoint_ref: &str) {
    let git_cmd = if let Some(git_root) = git_repo_root() {
        std::process::Command::new("git")
            .current_dir(git_root)
            .args(["branch", "-D", checkpoint_ref])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    } else {
        std::process::Command::new("git")
            .args(["branch", "-D", checkpoint_ref])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    };
    let _ = git_cmd;
    app_info!(
        "plan",
        "checkpoint",
        "Cleaned up checkpoint branch: {}",
        checkpoint_ref
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan_steps() {
        let md = "\
### Phase 1: Analysis
- [ ] Read config files at src/config.ts
- [x] Analyze CSS variables in theme.css
### Phase 2: Implementation
- [ ] Add ThemeProvider component
- [ ] Create toggle button";
        let steps = parse_plan_steps(md);
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0].phase, "Phase 1: Analysis");
        assert_eq!(steps[0].title, "Read config files at src/config.ts");
        assert_eq!(steps[0].status, PlanStepStatus::Pending);
        assert_eq!(steps[1].status, PlanStepStatus::Completed);
        assert_eq!(steps[2].phase, "Phase 2: Implementation");
        assert_eq!(steps[2].index, 2);
    }

    #[test]
    fn test_plan_mode_state_roundtrip() {
        assert_eq!(PlanModeState::from_str("planning"), PlanModeState::Planning);
        assert_eq!(PlanModeState::from_str("review"), PlanModeState::Review);
        assert_eq!(
            PlanModeState::from_str("executing"),
            PlanModeState::Executing
        );
        assert_eq!(PlanModeState::from_str("paused"), PlanModeState::Paused);
        assert_eq!(
            PlanModeState::from_str("completed"),
            PlanModeState::Completed
        );
        assert_eq!(PlanModeState::from_str("off"), PlanModeState::Off);
        assert_eq!(PlanModeState::from_str("unknown"), PlanModeState::Off);
        assert_eq!(PlanModeState::Planning.as_str(), "planning");
        assert_eq!(PlanModeState::Review.as_str(), "review");
        assert_eq!(PlanModeState::Paused.as_str(), "paused");
        assert_eq!(PlanModeState::Completed.as_str(), "completed");
    }
}
