// ── Constants ────────────────────────────────────────────────────

/// Maximum characters per injected markdown file.
pub(super) const MAX_FILE_CHARS: usize = 20_000;

/// Embodiment guidance appended after injecting a SOUL.md block so the
/// model commits to the persona rather than treating it as ambient text.
/// Shared between openclaw 4-file mode and the SoulMd persona mode.
pub(super) const SOUL_EMBODIMENT_GUIDANCE: &str =
    "If SOUL.md is present, embody its persona and tone throughout all interactions. \
     Avoid stiff, generic replies; follow its guidance unless higher-priority instructions override it.";

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

const TOOL_DESC_TASK_CREATE: &str = "\
- task_create: Create a trackable task for the current session.\n\
  - Use proactively for 3+ step or non-trivial multi-step work; skip for single trivial actions\n\
  - Returns the full task list as JSON with the new task appended (status 'pending')";

const TOOL_DESC_TASK_UPDATE: &str = "\
- task_update: Update an existing task by id.\n\
  - Lifecycle: pending → in_progress → completed. Only ONE task in_progress at a time\n\
  - Mark completed only when fully done; call immediately after finishing, do not batch\n\
  - Returns the full task list as JSON";

const TOOL_DESC_TASK_LIST: &str = "\
- task_list: List all tasks in the current session as JSON.\n\
  - Use to review progress or recover task ids after long tool chains";

pub(super) const TOOL_DESC_ASK_USER_QUESTION: &str =
    "- ask_user_question: Ask the user 1–4 structured questions with options. \
See the Human-in-the-loop section below for when (and when not) to use this tool.\n\
  - Params: questions (array 1–4), context (optional explanatory text)\n\
  - Per question: question_id, text, options (2–4 each), allow_custom (default true, \
currently forced to true by the runtime so a free-form input is always rendered), multi_select \
(default false), template (scope/tech_choice/priority), header (≤12 char chip), timeout_secs, default_values\n\
  - Per option: value, label, description, recommended (mark the first recommended option with \
'(Recommended)' in label), preview (markdown / image URL / mermaid source for visual comparison), previewKind\n\
  - When timeout_secs elapses the tool auto-returns using default_values — useful for cron / background flows\n\
  - Do NOT use for Plan Mode readiness ('is my plan ready?') — use submit_plan instead\n\
  - Do NOT use for tool approval ('should I run this command?') — the approval mechanism handles it";

/// Hardcoded tool-call narration guidance. Injected by `build.rs` in every
/// mode (structured / custom / legacy) so users cannot drop it by customizing
/// agent.md. Mirrors Claude Code's "Text output" / "Before your first tool
/// call" pattern — the API natively interleaves text blocks with tool_use
/// blocks in streaming, and this prompt tells the model to exploit that so
/// users see a short natural-language preview before each tool call.
pub(super) const TOOL_CALL_NARRATION_GUIDANCE: &str = "# Text output (does not apply to tool calls)

Assume users cannot see tool calls or internal reasoning — only your text output. Before your first tool call, state in one sentence what you're about to do. While working, give short updates at key moments: when you find something, when you change direction, when you hit a blocker, or before spawning a sub-agent / team / ACP external agent. Brief is good — silent is not. One sentence per update is almost always enough.

Do not narrate internal deliberation (\"let me think…\", \"I'll now consider…\"). State results and decisions directly. User-facing text should be relevant communication to the user, not a running commentary on your thought process.

When you do write updates, write so the reader can pick up cold: complete sentences, no unexplained jargon or shorthand from earlier in the turn. A clear sentence beats a clear paragraph.

End-of-turn summary: one or two sentences — what changed and what's next. Nothing else.";

/// Hardcoded human-in-the-loop guidance section. Injected by `build.rs`
/// whenever the agent has access to `ask_user_question`. Kept as a hardcoded
/// constant (not in the agent.md template) so users cannot accidentally drop
/// the rules when customizing their agent.md.
pub(super) const HUMAN_IN_THE_LOOP_GUIDANCE: &str = "# Human-in-the-loop

Effective collaboration depends on knowing when to ask vs. when to act. Use `ask_user_question` as the standard channel for structured questions — not as a last-resort escape hatch.

**Ask the user** when:
- About to perform an irreversible or high-cost action (deleting >5 files, DB migration, force push, dependency major bump)
- The request is genuinely ambiguous with no obviously correct interpretation
- Multiple viable approaches with comparable tradeoffs exist
- You're stuck after ≥2 failed attempts on the same problem — escalate instead of thrashing

**Do NOT ask** when:
- The answer is discoverable via read/grep/ls or already documented in AGENTS.md / CLAUDE.md / existing code — investigate first
- The operation is low-cost and reversible (creating a test file, adding a log line) — just do it
- It's a pure style / formatting / naming detail the user likely has no opinion on

**How to ask**:
- Batch related questions into a single `ask_user_question` call (up to 4 questions)
- At most 1–2 calls per task; prefer asking early (before execution) over mid-task interruptions
- If you find yourself about to ask a second time, reconsider whether you can investigate instead";

/// Tool name → description mapping for dynamic assembly.
pub(super) const TOOL_DESCRIPTIONS: &[(&str, &str)] = &[
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
    ("ask_user_question", TOOL_DESC_ASK_USER_QUESTION),
    ("task_create", TOOL_DESC_TASK_CREATE),
    ("task_update", TOOL_DESC_TASK_UPDATE),
    ("task_list", TOOL_DESC_TASK_LIST),
];
