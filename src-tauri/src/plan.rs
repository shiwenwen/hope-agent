use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
use tokio::sync::Mutex as TokioMutex;
use serde::{Deserialize, Serialize};
use anyhow::Result;

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
}

impl PlanMeta {
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
}

fn default_true() -> bool { true }

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

fn get_pending_questions()
    -> &'static TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<PlanQuestionAnswer>>>>
{
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

// ── Tool Restrictions ───────────────────────────────────────────

/// Tools denied in Plan Mode (file modification + creation tools).
pub const PLAN_MODE_DENIED_TOOLS: &[&str] = &[
    "write",
    "edit",
    "apply_patch",
    "canvas",
];

/// Tools that require user approval (ask) in Plan Mode.
pub const PLAN_MODE_ASK_TOOLS: &[&str] = &["exec"];

// ── System Prompt ───────────────────────────────────────────────

pub const PLAN_MODE_SYSTEM_PROMPT: &str = "\
# Plan Mode Active

You are in **Plan Mode**. Collaborate with the user to create a detailed implementation plan through interactive Q&A.

## Restrictions
- You **CANNOT** create, modify, or delete files (write, edit, apply_patch tools are disabled)
- You **CAN** read files, search code, browse the web, and analyze the codebase
- Shell commands (exec) require user approval before execution

## Planning Process
1. **Analyze Requirements**: Understand what the user wants
2. **Explore & Research**: Read files, search code, browse web as needed
3. **Ask Clarifying Questions**: Use the `plan_question` tool to ask structured questions with suggested options
   - Group related questions together (send multiple in one call)
   - Provide 2-5 suggested options per question with clear labels
   - Set `allow_custom=true` when the user might have a different idea
   - After receiving answers, you may ask follow-up questions if needed
4. **Submit Plan**: When ready, use `submit_plan` to submit the final plan

## Tools
- `plan_question`: Send structured questions to the user with suggested options (renders as interactive UI)
- `submit_plan`: Submit the final plan (title + markdown content)
- All read-only tools (read, search, glob, web_search, web_fetch, etc.)

## Plan Format (for submit_plan content)
## Background
<context: problem, motivation, constraints, expected outcome>

### Phase 1: <title>
- [ ] Step description (include file paths)
- [ ] Step description

### Phase 2: <title>
- [ ] Step description

## Guidelines
- Always start with a **Background** section
- Include specific file paths and function names
- Each step should be independently verifiable
- Group related changes into phases
- Consider testing as a separate phase
- Do NOT output the plan directly in chat messages — always use `submit_plan` tool";

/// System prompt injected during plan execution phase.
pub const PLAN_EXECUTING_SYSTEM_PROMPT_PREFIX: &str = "\
# Executing Plan

You are executing an approved plan. Follow the steps below in order.
After completing each step, call the `update_plan_step` tool to mark your progress:
- `update_plan_step(step_index=N, status=\"in_progress\")` when starting a step
- `update_plan_step(step_index=N, status=\"completed\")` when done
- `update_plan_step(step_index=N, status=\"failed\")` if a step fails
- `update_plan_step(step_index=N, status=\"skipped\")` if skipping

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
            let paused_at = meta.steps.iter()
                .position(|s| s.status == PlanStepStatus::InProgress)
                .or_else(|| meta.steps.iter().position(|s| s.status == PlanStepStatus::Pending));
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
        map.insert(session_id.to_string(), PlanMeta {
            session_id: session_id.to_string(),
            title: None,
            file_path,
            state,
            steps: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            paused_at_step: None,
        });
    }
}

pub async fn get_plan_meta(session_id: &str) -> Option<PlanMeta> {
    let map = store().read().await;
    map.get(session_id).cloned()
}

pub async fn update_plan_steps(session_id: &str, steps: Vec<PlanStep>) {
    let mut map = store().write().await;
    if let Some(meta) = map.get_mut(session_id) {
        meta.steps = steps;
        meta.updated_at = chrono::Utc::now().to_rfc3339();
    }
}

pub async fn update_step_status(session_id: &str, step_index: usize, status: PlanStepStatus, duration_ms: Option<u64>) {
    let mut map = store().write().await;
    if let Some(meta) = map.get_mut(session_id) {
        if let Some(step) = meta.steps.get_mut(step_index) {
            step.status = status;
            if duration_ms.is_some() {
                step.duration_ms = duration_ms;
            }
            meta.updated_at = chrono::Utc::now().to_rfc3339();
        }
    }
}

/// Restore plan state from DB on session load.
pub async fn restore_from_db(session_id: &str, plan_mode_str: &str) {
    let state = PlanModeState::from_str(plan_mode_str);
    if state == PlanModeState::Off {
        return;
    }
    // Load plan content from file if exists
    let steps = match load_plan_file(session_id) {
        Ok(Some(content)) => parse_plan_steps(&content),
        _ => Vec::new(),
    };
    let file_path = plan_file_path(session_id)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut map = store().write().await;
    map.insert(session_id.to_string(), PlanMeta {
        session_id: session_id.to_string(),
        title: None,
        file_path,
        state,
        steps,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        paused_at_step: None,
    });
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
    let short_id = &session_id[..8.min(session_id.len())];
    let date = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!("plan-{}-{}.md", short_id, date);
    Ok(plans_dir()?.join(filename))
}

/// Build the result file path for a session.
fn result_file_path(session_id: &str) -> Result<std::path::PathBuf> {
    let short_id = &session_id[..8.min(session_id.len())];
    let date = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!("result-{}-{}.md", short_id, date);
    Ok(plans_dir()?.join(filename))
}

pub fn save_plan_file(session_id: &str, content: &str) -> Result<String> {
    let dir = plans_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = plan_file_path(session_id)?;
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
        Ok(Some(std::fs::read_to_string(path)?))
    } else {
        Ok(None)
    }
}

pub fn delete_plan_file(session_id: &str) -> Result<()> {
    let path = plan_file_path(session_id)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Save execution result as a separate markdown file.
pub fn save_result_file(session_id: &str, plan_title: &str, steps: &[PlanStep], summary: &str) -> Result<String> {
    let dir = plans_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = result_file_path(session_id)?;

    let mut md = String::new();
    md.push_str(&format!("# 执行结果: {}\n\n", plan_title));
    md.push_str(&format!("> 执行时间: {}\n\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")));

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
        let duration = step.duration_ms
            .map(|ms| format!(" ({}ms)", ms))
            .unwrap_or_default();
        md.push_str(&format!("- {} {}{}\n", icon, step.title, duration));
    }

    let completed = steps.iter().filter(|s| s.status == PlanStepStatus::Completed).count();
    let failed = steps.iter().filter(|s| s.status == PlanStepStatus::Failed).count();
    let skipped = steps.iter().filter(|s| s.status == PlanStepStatus::Skipped).count();
    md.push_str(&format!("\n## 统计\n\n- 完成: {}\n- 失败: {}\n- 跳过: {}\n- 总计: {}\n",
        completed, failed, skipped, steps.len()));

    if !summary.is_empty() {
        md.push_str(&format!("\n## 总结\n\n{}\n", summary));
    }

    std::fs::write(&path, &md)?;
    Ok(path.to_string_lossy().to_string())
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
            let (checked, text) = if let Some(t) = rest.strip_prefix("x] ").or_else(|| rest.strip_prefix("X] ")) {
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
        assert_eq!(PlanModeState::from_str("executing"), PlanModeState::Executing);
        assert_eq!(PlanModeState::from_str("paused"), PlanModeState::Paused);
        assert_eq!(PlanModeState::from_str("completed"), PlanModeState::Completed);
        assert_eq!(PlanModeState::from_str("off"), PlanModeState::Off);
        assert_eq!(PlanModeState::from_str("unknown"), PlanModeState::Off);
        assert_eq!(PlanModeState::Planning.as_str(), "planning");
        assert_eq!(PlanModeState::Review.as_str(), "review");
        assert_eq!(PlanModeState::Paused.as_str(), "paused");
        assert_eq!(PlanModeState::Completed.as_str(), "completed");
    }
}
