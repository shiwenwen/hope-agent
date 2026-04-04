use crate::agent_config::{AgentDefinition, FilterConfig, PersonalityConfig};
use crate::skills;
use crate::user_config;

// ── Constants ────────────────────────────────────────────────────

/// Maximum characters per injected markdown file.
const MAX_FILE_CHARS: usize = 20_000;

// ── Per-Tool Descriptions ───────────────────────────────────────
// Each tool has its own detailed description with usage guidelines,
// best practices, and common pitfalls. Referenced by TOOL_DESCRIPTIONS
// array and assembled dynamically by build_tools_section().

const TOOL_DESC_EXEC: &str = "\
- exec: Execute shell commands and return output.\n\
  - Supports cwd, timeout (default 30min, max 2h), custom env vars\n\
  - Background execution: background=true or yield_ms for auto-backgrounding\n\
  - Docker sandbox isolation: sandbox=true for untrusted or risky commands\n\
  - IMPORTANT: Prefer dedicated tools over exec for common operations:\n\
    - Read files → use `read` (not cat/head/tail)\n\
    - Edit files → use `edit` (not sed/awk)\n\
    - Create files → use `write` (not echo/cat heredoc)\n\
    - Search content → use `grep` (not grep/rg command)\n\
    - Find files → use `find` (not find command)\n\
  - For long-running commands (builds, installs), use background=true then process(action='poll')\n\
  - Use absolute paths throughout; avoid cd unless user explicitly requests it\n\
  - When creating files/dirs, first verify parent directory exists with ls\n\
  - Quote file paths containing spaces with double quotes\n\
  - For sequential dependent commands, chain with && in a single call\n\
  - For independent commands, make separate parallel exec calls";

const TOOL_DESC_PROCESS: &str = "\
- process: Manage background exec sessions.\n\
  - Actions: list, poll (get new output), log (full output), write (stdin), kill, clear, remove\n\
  - Use after backgrounding a command with exec(background=true)\n\
  - Do not poll in a loop with sleep — you will be notified when the process completes";

const TOOL_DESC_READ: &str = "\
- read: Read file contents with line-based pagination (offset/limit).\n\
  - Default: up to 2000 lines from beginning of file\n\
  - When you know which part you need, only read that part (important for large files)\n\
  - Auto-detects image files (PNG/JPEG/GIF/WebP/BMP/TIFF) and returns base64; oversized images auto-resized\n\
  - For large PDFs (>10 pages), MUST specify pages parameter (max 20 pages per request)\n\
  - Can read Jupyter notebooks (.ipynb) with all cells and outputs\n\
  - Accepts both 'path' and 'file_path'\n\
  - IMPORTANT: Read files BEFORE proposing modifications — understand existing code first\n\
  - Can only read files, not directories. Use `ls` for directory listings";

const TOOL_DESC_WRITE: &str = "\
- write: Write content to a file (overwrites existing).\n\
  - Prefer `edit` tool for modifying existing files — it sends only the diff\n\
  - Only create new files when absolutely necessary — prefer editing existing files to prevent file bloat\n\
  - If overwriting an existing file, MUST read it first to understand current content\n\
  - Do NOT create documentation files (*.md) or README unless explicitly requested\n\
  - Accepts both 'path' and 'file_path'";

const TOOL_DESC_EDIT: &str = "\
- edit: Targeted search-replace edits (old_text → new_text).\n\
  - ALWAYS prefer over `write` for modifications — sends only the diff\n\
  - old_text must be unique in the file — provide more surrounding context if not unique\n\
  - Use replace_all=true to rename variables/strings across the entire file\n\
  - Empty new_text deletes the matched text\n\
  - Preserve exact indentation from the source file\n\
  - Accepts aliases: file_path, oldText/old_string, newText/new_string";

const TOOL_DESC_LS: &str = "\
- ls: List directory contents (sorted, with / and @ indicators).\n\
  - Supports ~ expansion, limit param, 50KB output cap\n\
  - Use to verify directory structure before creating files";

const TOOL_DESC_GREP: &str = "\
- grep: Search file contents with regex or literal patterns.\n\
  - ALWAYS use this tool for content search — never grep/rg via exec\n\
  - Respects .gitignore automatically\n\
  - Full regex syntax supported; literal braces need escaping (e.g., interface\\{\\})\n\
  - For patterns spanning multiple lines, use multiline=true\n\
  - Params: pattern (required), path, glob, ignore_case, literal, context, limit (default 100)\n\
  - For open-ended searches requiring multiple rounds, use subagent instead";

const TOOL_DESC_FIND: &str = "\
- find: Find files by glob pattern.\n\
  - ALWAYS use this tool for file search — never find via exec\n\
  - Respects .gitignore automatically\n\
  - Params: pattern (required), path, limit (default 1000)";

const TOOL_DESC_APPLY_PATCH: &str = "\
- apply_patch: Apply multi-file patches (add/update/delete/move files).\n\
  - Use *** Begin Patch / *** End Patch format with Add File, Update File, Delete File markers\n\
  - Update hunks use @@ context + -/+ line prefixes with 3-pass fuzzy matching\n\
  - Preferred for large-scale changes across multiple files";

const TOOL_DESC_WEB_SEARCH: &str = "\
- web_search: Search the web for information.\n\
  - Use when you need current information not available in the codebase\n\
  - Returns search results with titles, snippets, and URLs";

const TOOL_DESC_WEB_FETCH: &str = "\
- web_fetch: Fetch and extract content from a web page.\n\
  - Use after web_search to get full content of a specific page\n\
  - Returns cleaned text content extracted from HTML";

const TOOL_DESC_SAVE_MEMORY: &str = "\
- save_memory: Save information to persistent memory.\n\
  - Use when the user shares personal info, preferences, corrections, or says \"remember this\"\n\
  - Params: content (required), type (user/feedback/project/reference), tags (optional array), scope (global/agent)\n\
  - Do NOT save: ephemeral task details, code snippets, debugging steps, or anything derivable from the codebase";

const TOOL_DESC_RECALL_MEMORY: &str = "\
- recall_memory: Search persistent memories by keyword or semantic query.\n\
  - Use to recall user preferences, project context, or previously stored information\n\
  - Use when the user references something discussed before or you need prior context\n\
  - Params: query (required), type (optional filter), limit (default 10)\n\
  - With include_history=true: also searches past conversation history";

const TOOL_DESC_UPDATE_MEMORY: &str = "\
- update_memory: Update an existing memory entry's content or metadata.\n\
  - Use after recall_memory or memory_get to modify a specific memory\n\
  - Params: id (required), content, type, tags";

const TOOL_DESC_DELETE_MEMORY: &str = "\
- delete_memory: Delete a memory entry by ID.\n\
  - Use to remove outdated or incorrect memories\n\
  - Params: id (required)";

const TOOL_DESC_UPDATE_CORE_MEMORY: &str = "\
- update_core_memory: Update the agent's core memory (persistent instructions in memory.md).\n\
  - Use for standing instructions, persistent preferences, and recurring corrections\n\
  - Params: content (required), section (optional)";

const TOOL_DESC_MANAGE_CRON: &str = "\
- manage_cron: Create, list, update, or delete scheduled tasks.\n\
  - Actions: create, list, get, update, delete, run_now\n\
  - Cron expressions follow standard format (minute hour day month weekday)";

const TOOL_DESC_BROWSER: &str = "\
- browser: Interact with web pages via a headless browser.\n\
  - Supports navigation, screenshots, clicking, typing, and JavaScript execution\n\
  - Use for dynamic web pages that web_fetch cannot handle";

const TOOL_DESC_SEND_NOTIFICATION: &str = "\
- send_notification: Send a system notification to the user.\n\
  - Use for important alerts when the user may not be watching the conversation\n\
  - Params: title, body (required), sound (optional boolean)";

const TOOL_DESC_SUBAGENT: &str = "\
- subagent: Spawn and manage sub-agents to delegate tasks.\n\
  - Actions: spawn, check, list, result, kill, kill_all, steer, batch_spawn, wait_all, spawn_and_wait\n\
  - Sub-agents run asynchronously — results are auto-pushed when complete\n\
  - spawn_and_wait: spawn + wait up to foreground_timeout (default 30s, max 120s). If completes in time, returns result inline. Otherwise auto-backgrounds — result injected later\n\
  - Use steer to redirect a running sub-agent without killing it";

const TOOL_DESC_MEMORY_GET: &str = "\
- memory_get: Retrieve a specific memory entry by ID with full content and metadata.\n\
  - Use after recall_memory to get the complete details of a specific memory";

const TOOL_DESC_AGENTS_LIST: &str = "\
- agents_list: List all available agents with their descriptions and capabilities.\n\
  - Useful for choosing which agent to delegate tasks to via subagent";

const TOOL_DESC_SESSIONS_LIST: &str = "\
- sessions_list: List all chat sessions with metadata (title, agent, model, message count).\n\
  - Use to discover existing sessions for cross-session communication";

const TOOL_DESC_SESSION_STATUS: &str = "\
- session_status: Query detailed status of a specific session.\n\
  - Returns session metadata, current state, and activity info";

const TOOL_DESC_SESSIONS_HISTORY: &str = "\
- sessions_history: Get paginated chat history from a specific session.\n\
  - Params: session_id (required), limit (default 50), before_id (pagination cursor), include_tools (default false)\n\
  - Use to understand context from another session before sending messages";

const TOOL_DESC_SESSIONS_SEND: &str = "\
- sessions_send: Send a message to another session for cross-session communication.\n\
  - Params: session_id, message (required), wait (default false), timeout_secs (default 60)\n\
  - Use wait=true to block until the other session responds";

const TOOL_DESC_IMAGE: &str = "\
- image: Analyze an image file and return base64-encoded data for visual analysis.\n\
  - Supports PNG, JPEG, GIF, WebP, BMP, TIFF\n\
  - Use prompt param to specify what to analyze (e.g., \"describe the UI layout\")";

const TOOL_DESC_IMAGE_GENERATE: &str = "\
- image_generate: Generate images from text descriptions using AI image generation models.\n\
  - Params: prompt (required), size (default 1024x1024), n (1-4, default 1), model (optional, default auto with failover)\n\
  - Generated images are saved to disk and returned for visual inspection";

const TOOL_DESC_PDF: &str = "\
- pdf: Extract text content from PDF documents with page-level pagination.\n\
  - Params: path (required), pages (e.g. '1-5'), max_chars (default 50000)\n\
  - For large PDFs, always specify pages to avoid excessive output";

const TOOL_DESC_CANVAS: &str = "\
- canvas: Create and edit rich content artifacts (diagrams, documents, visualizations).\n\
  - Use for content that benefits from visual rendering";

const TOOL_DESC_ACP_SPAWN: &str = "\
- acp_spawn: Delegate tasks to external ACP-compatible agents (e.g., Claude Code, Codex).\n\
  - Similar to subagent but for external processes with their own tools and capabilities\n\
  - Actions: spawn, check, list, result, kill, kill_all, steer, backends";

const TOOL_DESC_GET_WEATHER: &str = "\
- get_weather: Get current weather and forecast for a location.\n\
  - Uses Open-Meteo API (free, no key required)\n\
  - Params: location (city name or lat,lon, optional — defaults to user's location), forecast_days (1-16)\n\
  - Returns current temperature, humidity, wind, weather conditions, and daily forecast";

/// Tool name → description mapping for dynamic assembly.
const TOOL_DESCRIPTIONS: &[(&str, &str)] = &[
    ("exec", TOOL_DESC_EXEC),
    ("process", TOOL_DESC_PROCESS),
    ("read", TOOL_DESC_READ),
    ("write", TOOL_DESC_WRITE),
    ("edit", TOOL_DESC_EDIT),
    ("ls", TOOL_DESC_LS),
    ("grep", TOOL_DESC_GREP),
    ("find", TOOL_DESC_FIND),
    ("apply_patch", TOOL_DESC_APPLY_PATCH),
    ("web_search", TOOL_DESC_WEB_SEARCH),
    ("web_fetch", TOOL_DESC_WEB_FETCH),
    ("save_memory", TOOL_DESC_SAVE_MEMORY),
    ("recall_memory", TOOL_DESC_RECALL_MEMORY),
    ("update_memory", TOOL_DESC_UPDATE_MEMORY),
    ("delete_memory", TOOL_DESC_DELETE_MEMORY),
    ("update_core_memory", TOOL_DESC_UPDATE_CORE_MEMORY),
    ("manage_cron", TOOL_DESC_MANAGE_CRON),
    ("browser", TOOL_DESC_BROWSER),
    ("send_notification", TOOL_DESC_SEND_NOTIFICATION),
    ("subagent", TOOL_DESC_SUBAGENT),
    ("memory_get", TOOL_DESC_MEMORY_GET),
    ("agents_list", TOOL_DESC_AGENTS_LIST),
    ("sessions_list", TOOL_DESC_SESSIONS_LIST),
    ("session_status", TOOL_DESC_SESSION_STATUS),
    ("sessions_history", TOOL_DESC_SESSIONS_HISTORY),
    ("sessions_send", TOOL_DESC_SESSIONS_SEND),
    ("image", TOOL_DESC_IMAGE),
    ("image_generate", TOOL_DESC_IMAGE_GENERATE),
    ("pdf", TOOL_DESC_PDF),
    ("canvas", TOOL_DESC_CANVAS),
    ("acp_spawn", TOOL_DESC_ACP_SPAWN),
    ("get_weather", TOOL_DESC_GET_WEATHER),
];

// ── Behavior Guidance ───────────────────────────────────────────

/// Output efficiency instructions — reduce LLM verbosity.
/// Reference: Claude Code system-prompt-output-efficiency.md
const BEHAVIOR_OUTPUT_EFFICIENCY: &str = "\
# Output Efficiency

IMPORTANT: Go straight to the point. Try the simplest approach first without going in circles.

Keep your text output brief and direct:
- Lead with the answer or action, not the reasoning
- Skip filler words, preamble, and unnecessary transitions
- Do not restate what the user said — just do it
- When explaining, include only what is necessary for the user to understand
- If you can say it in one sentence, don't use three

Focus text output on:
- Decisions that need the user's input
- High-level status updates at natural milestones
- Errors or blockers that change the plan

This does NOT apply to code or tool calls — only to your text responses.";

/// Action safety instructions — blast radius evaluation.
/// Reference: Claude Code system-prompt-executing-actions-with-care.md
const BEHAVIOR_ACTION_SAFETY: &str = "\
# Action Safety

Carefully consider the reversibility and blast radius of actions.

**Safe to execute freely** (local, reversible):
- Reading files, searching code, running tests
- Editing local files, creating branches

**Require user confirmation** (hard to reverse, affect shared systems):
- Destructive operations: deleting files/branches, dropping tables, rm -rf, overwriting uncommitted changes
- Hard-to-reverse operations: force-push, git reset --hard, amending published commits, removing packages
- Actions visible to others: pushing code, creating/commenting on PRs/issues, sending messages, posting to external services

When encountering obstacles:
- Do NOT use destructive actions as shortcuts — identify root causes first
- Investigate unexpected state (unfamiliar files, branches, config) before deleting or overwriting
- Prefer resolving merge conflicts over discarding changes
- If a lock file exists, investigate what process holds it rather than deleting it

Principle: measure twice, cut once.";

/// Task execution guidelines — how to approach work.
/// Reference: Claude Code system-prompt-doing-tasks-*.md
const BEHAVIOR_DOING_TASKS: &str = "\
# Task Execution Guidelines

- Do NOT propose changes to code you haven't read. Read files first, understand existing code before suggesting modifications.
- Do NOT create files unless absolutely necessary. Prefer editing existing files to prevent file bloat.
- When given an unclear instruction, interpret it in the context of software engineering and the current working directory. \
For example, if asked to change \"methodName\" to snake case, find the method in the code and modify it — don't just reply with the converted name.
- Avoid over-engineering:
  - Only make changes that are directly requested or clearly necessary
  - Don't add features, refactor code, or make \"improvements\" beyond what was asked
  - Don't add error handling for scenarios that can't happen — trust internal code and framework guarantees
  - Don't create helpers or abstractions for one-time operations
  - Three similar lines of code is better than a premature abstraction
- If your approach is blocked, consider alternative approaches rather than brute-forcing or retrying the same action repeatedly.
- Be careful not to introduce security vulnerabilities (XSS, SQL injection, command injection, etc.). If you notice insecure code you wrote, fix it immediately.";

// ── Build System Prompt ──────────────────────────────────────────

/// Build the complete system prompt from an AgentDefinition.
///
/// Assembly order (13 sections):
/// ① Identity line
/// ② agent.md — what this agent does
/// ③ persona.md — personality
/// ④ User context — from user.json
/// ⑤ tools.md — custom tool guidance
/// ⑥ Tool definitions — per-tool descriptions (filtered by agent config)
/// ⑦ Skills — available skill descriptions (filtered)
/// ⑦b Behavior guidance — output efficiency, action safety, task execution
/// ⑧ Memory — injected from memory backend
/// ⑨ Runtime info — date, OS, etc.
/// ⑩ Sub-agent delegation (conditional)
/// ⑪ Sandbox mode (conditional)
/// ⑫ (reserved for project context — not yet implemented)
/// ⑬ ACP external agents (conditional)
pub fn build(
    definition: &AgentDefinition,
    model: Option<&str>,
    provider: Option<&str>,
    memory_context: Option<&str>,
    agent_home: Option<&str>,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    if definition.config.use_custom_prompt {
        // ── Custom prompt mode: use markdown files directly, skip structured config ──

        // Minimal identity line
        sections.push(format!(
            "You are {}, running in OpenComputer on {} {}.",
            definition.config.name, os, arch
        ));

        // agent.md — custom identity / instructions
        if let Some(md) = definition
            .agent_md
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            sections.push(truncate(md, MAX_FILE_CHARS));
        }

        // persona.md — custom personality
        if let Some(persona) = definition
            .persona
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            sections.push(truncate(persona, MAX_FILE_CHARS));
        }
    } else {
        // ── Structured mode: assemble from config fields + optional supplements ──

        // ① Identity
        let role_suffix = definition
            .config
            .personality
            .role
            .as_deref()
            .filter(|r| !r.is_empty())
            .map(|r| format!(", a {}", r))
            .unwrap_or_default();
        sections.push(format!(
            "You are {}{}, running in OpenComputer on {} {}.",
            definition.config.name, role_suffix, os, arch
        ));

        // ② Personality (structured)
        let personality_section = build_personality_section(&definition.config.personality);
        if !personality_section.is_empty() {
            sections.push(personality_section);
        }

        // ③ agent.md — supplementary identity notes
        if let Some(md) = definition
            .agent_md
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            sections.push(truncate(md, MAX_FILE_CHARS));
        }

        // ④ persona.md — supplementary personality notes
        if let Some(persona) = definition
            .persona
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            sections.push(truncate(persona, MAX_FILE_CHARS));
        }
    }

    // ④ User context
    if let Ok(user_cfg) = user_config::load_user_config() {
        if let Some(user_section) = user_config::build_user_context(&user_cfg) {
            sections.push(user_section);
        }
    }

    // ⑤ tools.md
    if let Some(guide) = &definition.tools_guide {
        sections.push(truncate(guide, MAX_FILE_CHARS));
    }

    // ⑥ Tool definitions (filtered by agent config)
    sections.push(build_tools_section(&definition.config.tools));

    // ⑥b Deferred tools listing (when deferred loading is enabled)
    if let Some(deferred_section) = build_deferred_tools_section() {
        sections.push(deferred_section);
    }

    // ⑦ Skills (filtered by agent config)
    sections.push(build_skills_section(
        &definition.config.skills,
        definition.config.behavior.skill_env_check,
    ));

    // ⑦b Behavior guidance (output efficiency, action safety, task execution)
    sections.push(build_behavior_section());

    // ⑧ Memory
    if definition.config.memory.enabled {
        let mut memory_section = String::new();

        // 8a: Global Core Memory (shared across all agents)
        if let Some(md) = &definition.global_memory_md {
            if !md.trim().is_empty() {
                memory_section.push_str("## Core Memory (Global)\n\n");
                memory_section.push_str(&truncate(md, MAX_FILE_CHARS));
                memory_section.push_str("\n\n");
            }
        }

        // 8b: Agent Core Memory (specific to this agent)
        if let Some(md) = &definition.memory_md {
            if !md.trim().is_empty() {
                memory_section.push_str("## Core Memory (Agent)\n\n");
                memory_section.push_str(&truncate(md, MAX_FILE_CHARS));
                memory_section.push_str("\n\n");
            }
        }

        // 8c: SQLite memories (existing logic)
        if let Some(mem) = memory_context {
            if !mem.is_empty() {
                memory_section.push_str(mem);
                memory_section.push_str("\n\n");
            }
        }

        // 8d: Memory usage guidance
        memory_section.push_str(
            "## Memory Guidelines\n\
             Use update_core_memory when:\n\
             - The user gives a standing instruction (\"always\", \"never\", \"from now on\", \"remember to\")\n\
             - The user states a persistent preference or rule\n\
             - The user corrects a recurring behavior\n\n\
             Use save_memory when:\n\
             - You learn a fact about the user, project, or external resource\n\
             - The user mentions a deadline, event, or temporary context\n\
             - You discover something worth noting for future reference\n\n\
             Use recall_memory when:\n\
             - You need context about the user or project from prior conversations\n\
             - The user references something discussed before\n\
             - You want to check if preferences or constraints were previously established\n\n\
             Use recall_memory with include_history=true when:\n\
             - The user references a previous conversation (\"last time\", \"we discussed\", \"remember when\")\n\
             - You need to find what was said or decided in an earlier session\n\n\
             Do NOT save: ephemeral task details, code snippets, debugging steps, or anything derivable from the codebase."
        );

        if !memory_section.is_empty() {
            sections.push(memory_section);
        }
    }

    // ⑨ Runtime info
    sections.push(build_runtime_section(model, provider, agent_home));

    // ⑩ Sub-agent delegation (conditionally injected)
    if definition.config.subagents.enabled {
        let subagent_section =
            build_subagent_section(&definition.config.subagents, &definition.id, 0);
        if !subagent_section.is_empty() {
            sections.push(subagent_section);
        }
    }

    // ⑪ Sandbox mode (conditionally injected)
    if definition.config.behavior.sandbox {
        sections.push(
            "# Sandbox Mode\n\n\
             All commands you execute via the `exec` tool will automatically run inside a Docker sandbox container.\n\
             You do NOT need to pass `sandbox=true` — it is enforced by your agent configuration.\n\n\
             Sandbox properties:\n\
             - Read-only root filesystem (only /workspace, /tmp, /var/tmp, /run are writable)\n\
             - No network access (network mode: none)\n\
             - All Linux capabilities dropped\n\
             - Process count limited\n\
             - Your working directory is mounted at /workspace inside the container\n\n\
             If a command needs to write temporary files, use /tmp. \
             If a command requires network access or special privileges, inform the user that it cannot run in sandbox mode."
                .to_string(),
        );
    }

    // ⑬ ACP external agent delegation (conditionally injected)
    if definition.config.acp.enabled {
        let acp_section = build_acp_section();
        if !acp_section.is_empty() {
            sections.push(acp_section);
        }
    }

    // ⑫ Project context — not yet implemented

    // ⑭ Weather context (from cached weather data)
    if let Some(weather_text) = crate::weather::get_weather_for_prompt() {
        sections.push(weather_text);
    }

    // Join all non-empty sections
    let section_lengths: Vec<usize> = sections.iter().map(|s| s.len()).collect();
    let prompt = sections
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    // Log system prompt build result
    if let Some(logger) = crate::get_logger() {
        logger.log(
            "debug",
            "agent",
            "system_prompt::build",
            &format!(
                "System prompt built: {} chars, {} sections",
                prompt.len(),
                section_lengths.len()
            ),
            Some(
                serde_json::json!({
                    "total_length": prompt.len(),
                    "section_count": section_lengths.len(),
                    "section_lengths": section_lengths,
                    "agent_name": &definition.config.name,
                    "custom_prompt_mode": definition.config.use_custom_prompt,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    prompt
}

/// Build a system prompt using the legacy path (no AgentDefinition).
/// This preserves backward compatibility during the transition.
pub fn build_legacy(model: Option<&str>, provider: Option<&str>) -> String {
    let store = crate::provider::load_store().unwrap_or_default();
    let available_skills =
        skills::load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    let skills_section = skills::build_skills_prompt(
        &available_skills,
        &store.disabled_skills,
        store.skill_env_check,
        &store.skill_env,
        &store.skill_prompt_budget,
        &store.skill_allow_bundled,
    );

    let mut sections = Vec::new();

    // Identity
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    sections.push(format!(
        "You are OpenComputer, a personal AI assistant with deep system integration. \
         You help users interact with their computer naturally and efficiently. \
         Running on {} {}.",
        os, arch
    ));

    // User context
    if let Ok(user_cfg) = user_config::load_user_config() {
        if let Some(user_section) = user_config::build_user_context(&user_cfg) {
            sections.push(user_section);
        }
    }

    // Tools
    sections.push(build_all_tools_description());

    // Deferred tools listing
    if let Some(deferred_section) = build_deferred_tools_section() {
        sections.push(deferred_section);
    }

    // Skills
    if !skills_section.is_empty() {
        sections.push(skills_section);
    }

    // Behavior guidance
    sections.push(build_behavior_section());

    // Weather context
    if let Some(weather_text) = crate::weather::get_weather_for_prompt() {
        sections.push(weather_text);
    }

    // Runtime (legacy mode has no agent home)
    sections.push(build_runtime_section(model, provider, None));

    sections.join("\n\n")
}

// ── Section Builders ─────────────────────────────────────────────

/// Build tool definitions section, filtered by agent config.
/// Only includes descriptions for tools the agent is allowed to use.
fn build_tools_section(filter: &FilterConfig) -> String {
    let no_filter = filter.allow.is_empty() && filter.deny.is_empty();

    let descs: Vec<&str> = TOOL_DESCRIPTIONS
        .iter()
        .filter(|(name, _)| no_filter || filter.is_allowed(name))
        .map(|(_, desc)| *desc)
        .collect();

    if descs.is_empty() {
        return String::new();
    }

    format!("# Available Tools\n\n{}", descs.join("\n\n"))
}

/// Build a flat tool descriptions string for legacy mode (all tools).
fn build_all_tools_description() -> String {
    let descs: Vec<&str> = TOOL_DESCRIPTIONS.iter().map(|(_, desc)| *desc).collect();
    format!("# Available Tools\n\n{}", descs.join("\n\n"))
}

/// Build a section listing deferred tools (name + one-line description).
/// Only generated when deferred tool loading is enabled.
fn build_deferred_tools_section() -> Option<String> {
    let store = crate::provider::load_store().unwrap_or_default();
    if !store.deferred_tools.enabled {
        return None;
    }
    let deferred = crate::tools::get_deferred_tools();
    if deferred.is_empty() {
        return None;
    }
    let mut lines = vec![
        "# Additional Tools (use tool_search to discover)".to_string(),
        "The following tools are available but their schemas are not loaded by default. \
         Use `tool_search(query=\"keyword\")` to get the full schema before calling them."
            .to_string(),
        String::new(),
    ];
    for tool in &deferred {
        let short_desc = tool.description.split('.').next().unwrap_or(&tool.description);
        lines.push(format!("- **{}**: {}", tool.name, short_desc));
    }
    // Also include conditionally-injected deferred tools
    let extra_names = [
        crate::tools::TOOL_WEB_SEARCH,
        crate::tools::TOOL_SEND_NOTIFICATION,
        crate::tools::TOOL_IMAGE_GENERATE,
        crate::tools::TOOL_CANVAS,
        crate::tools::TOOL_ACP_SPAWN,
    ];
    for name in &extra_names {
        if !deferred.iter().any(|t| t.name == *name) {
            lines.push(format!("- **{}**: Use tool_search to discover", name));
        }
    }
    Some(lines.join("\n"))
}

/// Build behavior guidance section (output efficiency + action safety + task execution).
fn build_behavior_section() -> String {
    format!(
        "{}\n\n{}\n\n{}",
        BEHAVIOR_OUTPUT_EFFICIENCY, BEHAVIOR_ACTION_SAFETY, BEHAVIOR_DOING_TASKS,
    )
}

/// Build skills section, filtered by agent config.
fn build_skills_section(filter: &FilterConfig, env_check: bool) -> String {
    let store = crate::provider::load_store().unwrap_or_default();
    let all_skills =
        skills::load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);

    // Start with globally disabled skills
    let disabled = store.disabled_skills.clone();

    // Apply agent-level filtering
    let filtered: Vec<skills::SkillEntry> = all_skills
        .into_iter()
        .filter(|s| filter.is_allowed(&s.name))
        .collect();

    skills::build_skills_prompt(
        &filtered,
        &disabled,
        env_check,
        &store.skill_env,
        &store.skill_prompt_budget,
        &store.skill_allow_bundled,
    )
}

/// Build personality section from structured config.
fn build_personality_section(p: &PersonalityConfig) -> String {
    let mut lines: Vec<String> = Vec::new();

    if let Some(vibe) = &p.vibe {
        lines.push(format!("- Vibe: {}", vibe));
    }
    if let Some(tone) = &p.tone {
        lines.push(format!("- Tone: {}", tone));
    }
    if let Some(style) = &p.communication_style {
        lines.push(format!("- Communication style: {}", style));
    }
    if !p.traits.is_empty() {
        lines.push(format!("- Traits: {}", p.traits.join(", ")));
    }
    if !p.principles.is_empty() {
        lines.push("- Principles:".to_string());
        for principle in &p.principles {
            lines.push(format!("  - {}", principle));
        }
    }
    if let Some(boundaries) = &p.boundaries {
        lines.push(format!("- Boundaries: {}", boundaries));
    }
    if let Some(quirks) = &p.quirks {
        lines.push(format!("- Quirks: {}", quirks));
    }

    if lines.is_empty() {
        return String::new();
    }

    format!("# Personality\n\n{}", lines.join("\n"))
}

/// Build runtime information section.
fn build_runtime_section(
    model: Option<&str>,
    provider: Option<&str>,
    agent_home: Option<&str>,
) -> String {
    let now = current_date();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
    let os = format!("{} {}", std::env::consts::OS, os_version());
    let arch = std::env::consts::ARCH;
    let hostname = hostname();

    // Working directory: agent home if set, otherwise process cwd
    let working_dir = agent_home.map(|h| h.to_string()).unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    });
    let git_root = find_git_root(&working_dir);

    // Shared directory for cross-agent data
    let shared_dir = crate::paths::home_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    let mut lines = vec![
        format!("- Date: {} (use `date` command for exact time)", now),
        format!("- Host: {}", hostname),
        format!("- OS: {} ({})", os, arch),
        format!("- Shell: {}", shell),
        format!("- Working directory: {}", working_dir),
    ];

    if let Some(ref shared) = shared_dir {
        lines.push(format!(
            "- Shared directory: {} (shared across all agents — use for cross-agent data exchange)",
            shared
        ));
    }

    if let Some(root) = &git_root {
        lines.push(format!("- Git root: {}", root));
    }

    if let Some(m) = model {
        let label = match provider {
            Some(p) => format!("{}/{}", p, m),
            None => m.to_string(),
        };
        lines.push(format!("- Model: {}", label));
    }

    format!("# Runtime\n\n{}", lines.join("\n"))
}

/// Get OS version string via `uname -r`.
fn os_version() -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Get machine hostname.
fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Walk up from `start` to find the nearest `.git` directory.
fn find_git_root(start: &str) -> Option<String> {
    let mut dir = std::path::PathBuf::from(start);
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_string_lossy().to_string());
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Get current date as a stable string (date-only, no time).
/// Excludes time to maximize prompt cache hit rate — the system prompt
/// stays identical throughout the day. Agents can use `exec date` for
/// the precise time when needed.
fn current_date() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%d %Z")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Build sub-agent delegation section.
/// Only included when `SubagentConfig.enabled == true` and `depth < MAX_DEPTH`.
fn build_subagent_section(
    config: &crate::agent_config::SubagentConfig,
    current_agent_id: &str,
    depth: u32,
) -> String {
    let effective_max = config
        .max_spawn_depth
        .map(|d| d.clamp(1, 5))
        .unwrap_or(crate::subagent::max_depth());
    if depth >= effective_max {
        return String::new();
    }

    let mut lines = vec![
        "# Sub-Agent Delegation".to_string(),
        String::new(),
        "You can delegate tasks to other agents using the `subagent` tool.".to_string(),
    ];

    // List available agents for delegation
    let agents = crate::agent_loader::list_agents().unwrap_or_default();
    let available: Vec<_> = agents
        .iter()
        .filter(|a| a.id != current_agent_id) // Don't delegate to self
        .filter(|a| config.is_agent_allowed(&a.id))
        .collect();

    if !available.is_empty() {
        lines.push(String::new());
        lines.push("Available agents for delegation:".to_string());
        for a in &available {
            let desc = a.description.as_deref().unwrap_or("No description");
            let emoji = a.emoji.as_deref().unwrap_or("");
            lines.push(format!("- {} {} (id: `{}`): {}", emoji, a.name, a.id, desc));
        }
    }

    lines.push(String::new());
    lines.push("## How it works".to_string());
    lines.push(
        "1. Call `subagent(action=\"spawn\", task=\"...\", agent_id=\"...\")` to delegate a task"
            .to_string(),
    );
    lines.push(
        "2. The sub-agent runs **asynchronously** — you can continue working on other things"
            .to_string(),
    );
    lines.push("3. When the sub-agent completes, its result is **automatically pushed** to you as a new message".to_string());
    lines.push("4. If you need to actively wait: `subagent(action=\"check\", run_id=\"...\", wait=true)` blocks until done (fallback)".to_string());
    lines.push(String::new());
    lines.push("## Steer a running sub-agent".to_string());
    lines.push("- `subagent(action=\"steer\", run_id=\"...\", message=\"...\")` — inject a message to redirect a running sub-agent without killing it".to_string());
    lines.push(String::new());
    lines.push("## Other actions".to_string());
    lines.push(
        "- `subagent(action=\"check\", run_id=\"...\")` — quick status check (non-blocking)"
            .to_string(),
    );
    lines.push("- `subagent(action=\"list\")` — list all sub-agent runs".to_string());
    lines.push("- `subagent(action=\"kill\", run_id=\"...\")` — terminate a sub-agent".to_string());
    lines.push(String::new());
    lines.push("## Spawn options".to_string());
    lines.push("- `label`: display label for tracking (e.g., `label=\"research\"`)".to_string());
    lines
        .push("- `files`: file attachments `[{name, content, mime_type?, encoding?}]`".to_string());
    lines.push("- `model`: model override `\"provider_id/model_id\"`".to_string());
    lines.push(String::new());
    lines.push("Sub-agents run in isolated sessions with their own tools and context.".to_string());
    lines.push(format!("Current depth: {}/{}", depth, effective_max));

    lines.join("\n")
}

/// Build sub-agent section with explicit depth (called from subagent execution context).
#[allow(dead_code)]
pub fn build_subagent_section_with_depth(
    config: &crate::agent_config::SubagentConfig,
    current_agent_id: &str,
    depth: u32,
) -> String {
    build_subagent_section(config, current_agent_id, depth)
}

// ── ACP Section ─────────────────────────────────────────────────

/// Build the ACP external agent delegation section for the system prompt.
fn build_acp_section() -> String {
    // Check global config
    let store = match crate::provider::load_store() {
        Ok(s) => s,
        Err(_) => return String::new(),
    };

    if !store.acp_control.enabled {
        return String::new();
    }

    // Build available backends list from config
    let mut backend_lines = Vec::new();
    for b in &store.acp_control.backends {
        if !b.enabled {
            continue;
        }
        // Check if binary is available
        let available = if std::path::Path::new(&b.binary).is_absolute() {
            std::path::Path::new(&b.binary).exists()
        } else {
            crate::acp_control::registry::resolve_binary(&b.binary).is_some()
        };
        if available {
            backend_lines.push(format!("- {}: {} (binary: {})", b.id, b.name, b.binary));
        }
    }

    if backend_lines.is_empty() {
        return String::new();
    }

    format!(
        "# External Agent Delegation (ACP)\n\n\
         You can delegate tasks to external ACP-compatible agents using the `acp_spawn` tool.\n\
         These agents run as separate processes with their own tools, context, and capabilities.\n\n\
         Available ACP backends:\n\
         {}\n\n\
         When to use external agents vs sub-agents:\n\
         - Use `subagent` for tasks within OpenComputer's internal agent pool\n\
         - Use `acp_spawn` when you need an external agent's specific capabilities \
         (e.g., Claude Code's file editing, Codex's code generation)\n\n\
         Actions: spawn (start), check (poll/wait), list, result, kill, kill_all, steer (follow-up), backends (list available)\n\n\
         External agents run asynchronously. Use check(run_id, wait=true) to block until completion.",
        backend_lines.join("\n")
    )
}

// ── Truncation ───────────────────────────────────────────────────

/// Truncate text to a maximum length, preserving head (70%) and tail (20%).
fn truncate(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }

    let head_size = max_chars * 70 / 100;
    let tail_size = max_chars * 20 / 100;

    // Find safe char boundaries
    let head_end = text
        .char_indices()
        .take_while(|(i, _)| *i < head_size)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(head_size);

    let tail_start = text
        .char_indices()
        .rev()
        .take_while(|(i, _)| text.len() - *i <= tail_size)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(text.len() - tail_size);

    format!(
        "{}\n\n[... truncated {} characters ...]\n\n{}",
        &text[..head_end],
        text.len() - head_end - (text.len() - tail_start),
        &text[tail_start..]
    )
}
