use super::file_io::plans_dir;

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

/// Extra context appended to PLAN_MODE_SYSTEM_PROMPT when running as a sub-agent.
/// Reminds the LLM that the executing agent has NO exploration history.
pub(super) const PLAN_SUBAGENT_CONTEXT_NOTICE: &str = "\
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
