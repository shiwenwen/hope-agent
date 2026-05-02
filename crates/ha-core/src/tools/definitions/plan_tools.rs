use serde_json::json;

use super::super::{TOOL_ASK_USER_QUESTION, TOOL_ENTER_PLAN_MODE, TOOL_SUBMIT_PLAN};
use super::types::{CoreSubclass, ToolDefinition, ToolTier};

/// Tool for asking the user structured questions at any point in a conversation.
///
/// Available in any conversation (not only Plan Mode). Supports rich
/// markdown/image previews, per-question timeouts with default fall-backs,
/// IM channel native buttons, and persistence across app restarts.
pub fn get_ask_user_question_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_ASK_USER_QUESTION.into(),
        description: "Ask the user one or more structured questions with multiple-choice options. \
Use this whenever you need to clarify requirements, pick between approaches, or confirm a \
decision before continuing. Each question renders as an interactive UI in the desktop app, \
as native buttons in IM channels that support them (Telegram, Slack, Feishu, QQ, Discord, \
LINE, Google Chat), and as a text fallback (reply 1a/1b/2a) in the rest. \n\n\
Guidelines: 1–4 questions per call, 2–4 options per question. Prefer single-select. Mark your \
recommended choice as the first option with '(Recommended)' in the label. Use `preview` for \
mockups, code comparisons or diagram snippets. Set `default_values` + `timeout_secs` when the \
answer can safely fall back (useful for cron / background / IM async flows). Do NOT use this \
tool to ask 'is my plan ready?' — in Plan Mode use `submit_plan` instead."
            .into(),
        tier: ToolTier::Core {
            subclass: CoreSubclass::Interaction,
        },
        internal: true,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "List of questions to ask the user (1-4 recommended)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question_id": {
                                "type": "string",
                                "description": "Unique identifier for this question (e.g. 'q_framework', 'q_scope')"
                            },
                            "text": {
                                "type": "string",
                                "description": "The question text to display to the user. Should end with '?'."
                            },
                            "header": {
                                "type": "string",
                                "description": "Very short chip/tag label (max ~12 chars) shown next to the question, e.g. 'Auth', 'Framework', 'Scope'"
                            },
                            "options": {
                                "type": "array",
                                "description": "Suggested options (2-4 recommended). A free-form custom input is also rendered alongside the options so the user can reply with a value you didn't list.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "value": { "type": "string", "description": "Option identifier" },
                                        "label": { "type": "string", "description": "Display text (1-5 words)" },
                                        "description": { "type": "string", "description": "Additional explanation of the option or its trade-offs" },
                                        "recommended": { "type": "boolean", "description": "Mark as recommended (renders with ★ badge). Put recommended option first.", "default": false },
                                        "preview": { "type": "string", "description": "Optional rich preview body for visual comparison: markdown (code/tables), image URL, or mermaid source. Displayed side-by-side with the option list." },
                                        "previewKind": { "type": "string", "description": "Preview kind: 'markdown' (default), 'image', or 'mermaid'", "enum": ["markdown", "image", "mermaid"] }
                                    },
                                    "required": ["value", "label"]
                                }
                            },
                            "allow_custom": {
                                "type": "boolean",
                                "description": "Whether to show a free-form custom input field. Currently always treated as true by the runtime regardless of the value sent — kept in the schema for forward compatibility.",
                                "default": true
                            },
                            "multi_select": {
                                "type": "boolean",
                                "description": "Whether the user can select multiple options (default: false)",
                                "default": false
                            },
                            "template": {
                                "type": "string",
                                "description": "Optional UI category: 'scope', 'tech_choice', 'priority'",
                                "enum": ["scope", "tech_choice", "priority"]
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "description": "Per-question timeout in seconds. When exceeded, default_values are auto-applied. 0 or missing = use global default.",
                                "minimum": 0
                            },
                            "default_values": {
                                "type": "array",
                                "description": "Option values used automatically if the question times out. Each entry must be an existing option value, or a free-form custom string.",
                                "items": { "type": "string" }
                            }
                        },
                        "required": ["question_id", "text", "options"]
                    }
                },
                "context": {
                    "type": "string",
                    "description": "Optional context text explaining why these questions are being asked"
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        }),
    }
}

/// Tool the model uses to proactively enter Plan Mode before tackling a
/// non-trivial task.
pub fn get_enter_plan_mode_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_ENTER_PLAN_MODE.into(),
        description:
            "Use this tool **proactively** when you're about to start a non-trivial task. \
Getting user sign-off on the approach before producing the deliverable prevents wasted effort \
and ensures alignment. After entering, you can read files / search / use ask_user_question to \
clarify approach and preferences, then call `submit_plan` with the finalized plan when ready. \
The user reviews and approves the plan at submit time.\n\n\
## When to Use\n\n\
**Prefer using enter_plan_mode** for production-or-deliverable tasks unless they're simple. \
Use it when ANY of these conditions apply:\n\n\
1. **New Feature / Working Artifact**: Producing something the user will run, read, or interact \
with — even if it's a single file.\n\
   - Example: \"做一个网页版贪吃蛇\" — visual style, controls, scope all matter\n\
   - Example: \"Add a logout button\" — placement, styling, on-click behavior\n\
   - Example: \"写一篇 X 主题的文章\" — audience, tone, depth, structure all matter\n\n\
2. **Multiple Valid Approaches**: The task can be solved in several different ways with \
comparable trade-offs.\n\
   - Example: \"Add caching\" — Redis vs in-memory vs file-based\n\
   - Example: \"Implement state management\" — Redux vs Context vs custom\n\n\
3. **Code / Design Modifications**: Changes that affect existing behavior or structure.\n\
   - Example: \"Update the login flow\" — what exactly changes?\n\
   - Example: \"Refactor this component\" — what's the target architecture?\n\n\
4. **Multi-File / Multi-Section Changes**: The task likely touches 3+ files OR produces a \
deliverable with 3+ logical sections.\n\
   - Example: \"Refactor the authentication system\"\n\
   - Example: \"Add a new API endpoint with tests\"\n\n\
5. **Unclear Requirements**: You need to explore before understanding the full scope.\n\
   - Example: \"Make the app faster\" — need to profile first\n\
   - Example: \"Fix the checkout bug\" — need to investigate root cause\n\n\
6. **User Preferences Matter**: The implementation could reasonably go multiple ways and the \
user-facing experience depends on which way you pick.\n\
   - Visual style (pixel / minimal / skeuomorphic), control scheme (arrow keys / WASD / touch), \
tone (formal / casual), color palette, scope (MVP / full-featured), depth.\n\
   - **If you would use ask_user_question to clarify the approach or preferences, use \
enter_plan_mode instead** — plan mode is the right place to gather preferences then present a \
vetted plan, rather than making best-guess decisions and producing the artifact blindly.\n\n\
7. **Non-Code Domains** (writing / research / analysis / information organization): \
Same bar — if the deliverable has multiple reasonable directions, plan first.\n\n\
## When NOT to Use\n\n\
Only skip enter_plan_mode for genuinely simple tasks:\n\
- Single-line / few-line fixes (typos, obvious bugs, small tweaks)\n\
- Adding a single function with clear, fully-specified requirements\n\
- Tasks where the user has given very specific, detailed step-by-step instructions\n\
- Pure Q&A or research lookups (\"What does X do?\" — answer directly, don't plan)\n\
- One-off scripts with a single obvious output format (\"convert these CSVs to JSON\")\n\n\
## Examples\n\n\
### GOOD — Use enter_plan_mode:\n\n\
- \"做一个网页贪吃蛇\" — single file but visual / controls / scope are real choices\n\
- \"做一个登录页\" — placement, fields, validation strictness, error states\n\
- \"实现深色模式\" — affects many components, theme architecture decision\n\
- \"写一篇关于 X 的科普文章\" — tone, audience, depth, structure\n\
- \"调研 3 种向量数据库选型\" — comparison criteria, depth\n\n\
### BAD — Skip enter_plan_mode:\n\n\
- \"修一下 README 里的拼写错误\"\n\
- \"给 X 函数加一行 log\"\n\
- \"把变量 foo 改成 bar\"\n\
- \"X 函数是干什么的？\" (Q&A — just answer)\n\
- \"运行 cargo test\"\n\n\
## Important Notes\n\n\
- **If unsure whether to use it, err on the side of planning** — it's better to get alignment \
upfront (one Yes/No prompt) than to produce the wrong thing and rework.\n\
- Users appreciate being consulted before non-trivial work — \"best-guess once, regret all the \
way\" is much costlier than a 5-second prompt.\n\
- Calling this tool surfaces a Yes/No prompt. If the user accepts, the session transitions to \
Planning; if they decline, stay in normal mode and proceed directly. Only call this once per \
task; if declined, do not retry."
                .into(),
        tier: ToolTier::Core {
            subclass: CoreSubclass::PlanMode,
        },
        internal: true,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "One-line reason explaining why this task benefits from a written plan. Shown to the user as context for the Yes/No prompt."
                }
            },
            "additionalProperties": false
        }),
    }
}

/// Tool for submitting the final plan after interactive Q&A.
pub fn get_submit_plan_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_SUBMIT_PLAN.into(),
        description:
            "Submit the finalized plan as the design contract for the user to review and approve. \
The plan is a stable design document — once approved, it is frozen for the duration of execution. \
To revise an approved plan, exit plan mode and re-enter it.\n\n\
Recommended structure (any markdown is accepted, sections are guidance not enforcement):\n\
- Context — why this change is being made\n\
- Approach — the recommended approach (no alternatives needed)\n\
- Files — paths of critical files to be modified\n\
- Reuse — existing functions/utilities to reuse, with file paths\n\
- Verification — how to confirm the work was done correctly\n\n\
Do NOT include progress markers (no checkboxes, no status emojis, no \"TODO/DONE\" annotations). \
Progress is tracked separately via the task_create / task_update tools after the plan is approved."
                .into(),
        tier: ToolTier::Core {
            subclass: CoreSubclass::PlanMode,
        },
        internal: true,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short title for the plan (e.g. 'Refactor Auth Module')"
                },
                "content": {
                    "type": "string",
                    "description": "Plan content in markdown. Free-form structure (Context / Approach / Files / Reuse / Verification recommended). Do not include progress markers — those belong in task_* tools."
                }
            },
            "required": ["title", "content"],
            "additionalProperties": false
        }),
    }
}
