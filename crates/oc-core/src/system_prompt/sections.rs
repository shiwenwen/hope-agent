use super::constants::*;
use super::helpers::{current_date, find_git_root, hostname, os_version};
use crate::agent_config::{FilterConfig, PersonalityConfig};
use crate::project::{Project, ProjectFile};
use crate::skills;

// ── Section Builders ─────────────────────────────────────────────

/// Build tool definitions section, filtered by agent config.
/// Only includes descriptions for tools the agent is allowed to use.
pub(super) fn build_tools_section(filter: &FilterConfig) -> String {
    let no_filter = filter.allow.is_empty() && filter.deny.is_empty();

    let descs: Vec<&str> = TOOL_DESCRIPTIONS
        .iter()
        .filter(|(name, _)| no_filter || crate::tools::agent_tool_filter_allows(name, filter))
        .map(|(_, desc)| *desc)
        .collect();

    if descs.is_empty() {
        return String::new();
    }

    format!("# Available Tools\n\n{}", descs.join("\n\n"))
}

/// Build a flat tool descriptions string for legacy mode (all tools).
pub(super) fn build_all_tools_description() -> String {
    let descs: Vec<&str> = TOOL_DESCRIPTIONS.iter().map(|(_, desc)| *desc).collect();
    format!("# Available Tools\n\n{}", descs.join("\n\n"))
}

/// Build a section listing deferred tools (name + one-line description).
/// Only generated when deferred tool loading is enabled.
pub(super) fn build_deferred_tools_section() -> Option<String> {
    let store = crate::config::cached_config();
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
        let short_desc = tool
            .description
            .split('.')
            .next()
            .unwrap_or(&tool.description);
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

/// Build the async-tools usage guide section. Emitted whenever the global
/// `async_tools` feature is enabled — the model needs the `job_status` /
/// `<tool-job-result>` vocabulary regardless of agent-level policy.
pub(super) fn build_async_tools_section() -> Option<String> {
    let store = crate::config::cached_config();
    if !store.async_tools.enabled {
        return None;
    }
    let auto_bg = store.async_tools.auto_background_secs;
    let auto_bg_line = if auto_bg == 0 {
        "Auto-backgrounding is disabled in this environment.".to_string()
    } else {
        format!(
            "Sync calls to async-capable tools that exceed {auto_bg}s are auto-detached into background \
             jobs (status `auto_backgrounded`)."
        )
    };
    Some(format!(
        "# Async Tool Execution\n\n\
         Some tools (`exec`, `web_search`, `image_generate`) are **async-capable**: they accept an \
         optional `run_in_background: true` parameter that detaches the call into a background job \
         and returns immediately with a synthetic `{{job_id, status: \"started\"}}` response. The \
         conversation can continue while the job runs, and the real result is auto-injected back \
         into the chat as a `<tool-job-result job-id=\"...\">` user message when the session is idle.\n\n\
         **Use `run_in_background: true` when:**\n\
         - The task is expected to take more than a few seconds (long builds, slow web searches, \
           image generation, network-heavy operations), AND\n\
         - You can make progress on other things while it runs, OR\n\
         - The user explicitly asked you to continue working in parallel.\n\n\
         **Keep the call synchronous (default) when:** you need the result to decide your very next step.\n\n\
         **Polling:** call `job_status(job_id, block?: bool, timeout_ms?: number)` to inspect or \
         actively wait on a job. With `block: true` the call sleeps until the job reaches a terminal \
         state or `timeout_ms` elapses (default 60000, max 600000).\n\n\
         **Result injection:** when the job finishes, you'll see a `[Tool Job Completion — auto-delivered]` \
         user message containing a `<tool-job-result job-id=\"...\" status=\"...\">` block. Match the \
         `job-id` against the original synthetic response to associate the result with the original call.\n\n\
         {auto_bg_line}"
    ))
}

/// Build skills section, filtered by agent config.
pub(super) fn build_skills_section(filter: &FilterConfig, env_check: bool) -> String {
    let store = crate::config::cached_config();
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
pub(super) fn build_personality_section(p: &PersonalityConfig) -> String {
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
pub(super) fn build_runtime_section(
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

/// Build sub-agent delegation section.
/// Only included when `SubagentConfig.enabled == true` and `depth < MAX_DEPTH`.
pub(super) fn build_subagent_section(
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

    // List available agents for delegation (including self for forking)
    let agents = crate::agent_loader::list_agents().unwrap_or_default();
    let available: Vec<_> = agents
        .iter()
        .filter(|a| config.is_agent_allowed(&a.id))
        .collect();

    if !available.is_empty() {
        lines.push(String::new());
        lines.push("Available agents for delegation:".to_string());
        for a in &available {
            let desc = a.description.as_deref().unwrap_or("No description");
            let emoji = a.emoji.as_deref().unwrap_or("");
            let self_tag = if a.id == current_agent_id {
                " *(self — fork for parallel work)*"
            } else {
                ""
            };
            lines.push(format!(
                "- {} {} (id: `{}`): {}{}",
                emoji, a.name, a.id, desc, self_tag
            ));
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
    lines.push(String::new());
    lines.push("## Self-fork".to_string());
    lines.push(format!(
        "You can spawn yourself (`agent_id=\"{}\"`') as a fork for parallel work.",
        current_agent_id
    ));
    lines.push("Use this when a task has independent sub-tasks that benefit from parallel execution (e.g., modifying frontend and backend simultaneously).".to_string());
    lines.push(format!(
        "Do NOT self-fork for simple or sequential tasks. Depth limit: {}/{}.",
        depth, effective_max
    ));

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
pub(super) fn build_acp_section() -> String {
    // Check global config
    let store = crate::config::cached_config();
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

// ── Project sections ────────────────────────────────────────────

/// Build a "Current Project" section describing the project this session
/// belongs to: name, optional description, and optional custom instructions.
///
/// Injected into the system prompt right before the Memory section so the
/// LLM is primed with project context before reading project memories.
pub(super) fn build_project_context_section(project: &Project) -> String {
    let mut out = String::from("# Current Project\n\n");

    let title = match &project.emoji {
        Some(e) if !e.trim().is_empty() => format!("{} **{}**", e, project.name),
        _ => format!("**{}**", project.name),
    };
    out.push_str(&format!(
        "You are currently working inside project {}.\n",
        title
    ));

    if let Some(desc) = project
        .description
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str(&format!("\nDescription: {}\n", desc));
    }

    if let Some(instr) = project
        .instructions
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str("\n## Project Instructions\n\n");
        out.push_str(&super::helpers::truncate(instr, MAX_FILE_CHARS));
        out.push('\n');
    }

    out.push_str(
        "\nAll memories, files, and context below that live inside this project are \
         shared across every session in it. Memories you save will default to the \
         project scope unless you explicitly choose another scope.\n",
    );

    out
}

/// Build a "Project Files" section listing uploaded project files (Layer 1:
/// directory catalog), plus optionally inlining small files whose full
/// content fits inside the inline byte budget (Layer 2).
///
/// The LLM can read any listed file on demand via the `project_read_file`
/// tool (Layer 3), which is enforced to only open files within the current
/// session's project.
pub(super) fn build_project_files_section(
    project_id: &str,
    files: &[ProjectFile],
    inline_budget_bytes: usize,
) -> String {
    if files.is_empty() {
        return String::new();
    }

    let mut out = String::from("# Project Files\n\n");
    out.push_str(
        "The following files are shared across every session in this project. \
         Use `project_read_file(file_id=..., offset=..., limit=...)` (or `name=...`) \
         to open any of them on demand.\n\n",
    );

    // Layer 1: catalog — always emitted, cheap.
    out.push_str("## Available Files\n\n");
    for f in files {
        let size_kb = f.size_bytes as f64 / 1024.0;
        let icon = file_icon_for_mime(f.mime_type.as_deref());
        let extracted_note = match f.extracted_chars {
            Some(n) if n > 0 => format!("{} chars extracted", n),
            _ => "binary — not readable as text".to_string(),
        };
        out.push_str(&format!(
            "- {} **{}** — {:.1} KB, {}  \n  `file_id: {}`\n",
            icon, f.name, size_kb, extracted_note, f.id
        ));
    }

    // Layer 2: inline small text files up to the byte budget.
    let mut inlined_bytes = 0usize;
    let mut inline_buf = String::new();
    for f in files {
        let ext_path = match &f.extracted_path {
            Some(p) if !p.is_empty() => p,
            _ => continue,
        };
        let chars = f.extracted_chars.unwrap_or(0);
        if chars <= 0 || chars > 4096 {
            continue;
        }
        let base = match crate::paths::projects_dir() {
            Ok(d) => d,
            Err(_) => break,
        };
        let full = base.join(ext_path);
        let text = match std::fs::read_to_string(&full) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if inlined_bytes + text.len() > inline_budget_bytes {
            continue;
        }
        inline_buf.push_str(&format!(
            "\n### {} (full content)\n\n```\n{}\n```\n",
            f.name, text
        ));
        inlined_bytes += text.len();
    }

    if !inline_buf.is_empty() {
        out.push_str("\n## Inlined Small Files\n");
        out.push_str(&inline_buf);
    }

    let _ = project_id; // currently unused; reserved for per-project budget overrides
    out
}

/// Pick a short emoji/icon label for the given MIME type. Keeps the catalog
/// compact without pulling in an actual icon font.
fn file_icon_for_mime(mime: Option<&str>) -> &'static str {
    let mime = mime.unwrap_or("");
    if mime.starts_with("image/") {
        "🖼️"
    } else if mime.starts_with("audio/") {
        "🎵"
    } else if mime.starts_with("video/") {
        "🎬"
    } else if mime == "application/pdf" {
        "📄"
    } else if mime.contains("word") || mime.contains("officedocument") {
        "📝"
    } else if mime.contains("spreadsheet") || mime.contains("excel") {
        "📊"
    } else if mime.contains("zip") || mime.contains("compressed") || mime.contains("tar") {
        "🗜️"
    } else if mime.starts_with("text/") || mime.contains("json") || mime.contains("xml") {
        "📃"
    } else {
        "📁"
    }
}
