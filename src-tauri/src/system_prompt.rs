use crate::agent_config::{AgentDefinition, FilterConfig, PersonalityConfig};
use crate::skills;
use crate::user_config;

// ── Constants ────────────────────────────────────────────────────

/// Maximum characters per injected markdown file.
const MAX_FILE_CHARS: usize = 20_000;

/// Tool descriptions — kept here as the canonical reference.
/// Previously hardcoded in agent.rs as SYSTEM_PROMPT_BASE.
const TOOLS_DESCRIPTION: &str = "\
Available tools: \
- exec: Execute shell commands. Supports cwd, timeout (default 30min, max 2h), \
custom env vars, background execution (background=true or yield_ms for auto-backgrounding), \
and Docker sandbox isolation (sandbox=true) for untrusted or risky commands. \
- process: Manage background exec sessions — list, poll (get new output), log (full output), \
write (stdin), kill, clear, remove. Use after backgrounding a command. \
- read: Read file contents with line-based pagination (offset/limit). \
Auto-detects image files (PNG/JPEG/GIF/WebP/BMP/TIFF) and returns base64. \
Oversized images are auto-resized. Accepts both 'path' and 'file_path'. \
- write: Write content to a file. Accepts both 'path' and 'file_path'. \
- edit: Targeted search-replace edits (old_text → new_text). Prefer over write for modifications. \
Accepts aliases: file_path, oldText/old_string, newText/new_string. Empty new_text deletes text. \
- ls: List directory contents (sorted, with / and @ indicators). Supports ~ expansion, limit param, 50KB output cap. \
- grep: Search file contents with regex or literal patterns. Respects .gitignore. \
Params: pattern (required), path, glob, ignore_case, literal, context, limit (default 100). \
- find: Find files by glob pattern. Respects .gitignore. \
Params: pattern (required), path, limit (default 1000). \
- apply_patch: Apply multi-file patches (add/update/delete/move files). \
Use *** Begin Patch / *** End Patch format with Add File, Update File, Delete File markers. \
Update hunks use @@ context + -/+ line prefixes with 3-pass fuzzy matching. \
- web_search / web_fetch: Search the web and fetch page content. \
- save_memory: Save information to persistent memory. Use when the user shares personal info, \
preferences, corrections, project context, or says \"remember this\". \
Params: content (required), type (user/feedback/project/reference), tags (optional array), scope (global/agent). \
- recall_memory: Search persistent memories by keyword or semantic query. \
Use to recall user preferences, project context, or previously stored information. \
Params: query (required), type (optional filter), limit (default 10). \
\
For long-running commands (builds, installs), consider using background=true and then \
process(action='poll') to check progress.";

// ── Build System Prompt ──────────────────────────────────────────

/// Build the complete system prompt from an AgentDefinition.
///
/// Assembly order (10 sections):
/// ① Identity line
/// ② agent.md — what this agent does
/// ③ persona.md — personality
/// ④ User context — from user.json
/// ⑤ tools.md — custom tool guidance
/// ⑥ Tool definitions — built-in tool descriptions (filtered)
/// ⑦ Skills — available skill descriptions (filtered)
/// ⑧ Memory — injected from memory backend
/// ⑨ Runtime info — date, OS, etc.
/// ⑩ (reserved for project context — not yet implemented)
pub fn build(definition: &AgentDefinition, model: Option<&str>, provider: Option<&str>, memory_context: Option<&str>, agent_home: Option<&str>) -> String {
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
        if let Some(md) = &definition.agent_md {
            sections.push(truncate(md, MAX_FILE_CHARS));
        }

        // persona.md — custom personality
        if let Some(persona) = &definition.persona {
            sections.push(truncate(persona, MAX_FILE_CHARS));
        }
    } else {
        // ── Structured mode: assemble from config fields + optional supplements ──

        // ① Identity
        let role_suffix = definition.config.personality.role
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
        if let Some(md) = &definition.agent_md {
            sections.push(truncate(md, MAX_FILE_CHARS));
        }

        // ④ persona.md — supplementary personality notes
        if let Some(persona) = &definition.persona {
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

    // ⑦ Skills (filtered by agent config)
    sections.push(build_skills_section(&definition.config.skills, definition.config.behavior.skill_env_check));

    // ⑧ Memory
    if definition.config.memory.enabled {
        let mut memory_section = String::new();

        // Existing memories
        if let Some(mem) = memory_context {
            if !mem.is_empty() {
                memory_section.push_str(mem);
                memory_section.push_str("\n\n");
            }
        }

        // Memory usage guidance
        memory_section.push_str(
            "## Memory Guidelines\n\
             Use save_memory when:\n\
             - The user shares personal info (name, role, preferences, expertise)\n\
             - The user corrects your behavior or says \"don't do X\" / \"always do Y\"\n\
             - The user mentions project context, deadlines, or architecture decisions\n\
             - The user explicitly says \"remember this\" or \"don't forget\"\n\
             - You learn something important that would help in future conversations\n\n\
             Use recall_memory when:\n\
             - You need context about the user or project from prior conversations\n\
             - The user references something discussed before\n\
             - You want to check if preferences or constraints were previously established\n\n\
             Do NOT save: ephemeral task details, code snippets, debugging steps, or anything derivable from the codebase."
        );

        if !memory_section.is_empty() {
            sections.push(memory_section);
        }
    }

    // ⑨ Runtime info
    sections.push(build_runtime_section(model, provider, agent_home));

    // ⑩ Project context — not yet implemented

    // Join all non-empty sections
    let section_lengths: Vec<usize> = sections.iter().map(|s| s.len()).collect();
    let prompt = sections
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    // Log system prompt build result
    if let Some(logger) = crate::get_logger() {
        logger.log("debug", "agent", "system_prompt::build",
            &format!("System prompt built: {} chars, {} sections", prompt.len(), section_lengths.len()),
            Some(serde_json::json!({
                "total_length": prompt.len(),
                "section_count": section_lengths.len(),
                "section_lengths": section_lengths,
                "agent_name": &definition.config.name,
                "custom_prompt_mode": definition.config.use_custom_prompt,
            }).to_string()),
            None, None);
    }

    prompt
}

/// Build a system prompt using the legacy path (no AgentDefinition).
/// This preserves backward compatibility during the transition.
pub fn build_legacy(model: Option<&str>, provider: Option<&str>) -> String {
    let store = crate::provider::load_store().unwrap_or_default();
    let available_skills = skills::load_all_skills_with_extra(&store.extra_skills_dirs);
    let skills_section = skills::build_skills_prompt(&available_skills, &store.disabled_skills, store.skill_env_check, &store.skill_env);

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
    sections.push(TOOLS_DESCRIPTION.to_string());

    // Skills
    if !skills_section.is_empty() {
        sections.push(skills_section);
    }

    // Runtime (legacy mode has no agent home)
    sections.push(build_runtime_section(model, provider, None));

    sections.join("\n\n")
}

// ── Section Builders ─────────────────────────────────────────────

/// Build tool definitions section, filtered by agent config.
fn build_tools_section(filter: &FilterConfig) -> String {
    // If no filtering configured, return full descriptions
    if filter.allow.is_empty() && filter.deny.is_empty() {
        return TOOLS_DESCRIPTION.to_string();
    }

    // All tool names in the system
    let all_tools = [
        "exec", "process", "read", "write", "edit",
        "ls", "grep", "find", "apply_patch", "web_search", "web_fetch",
        "save_memory", "recall_memory",
    ];

    let active: Vec<&&str> = all_tools.iter().filter(|t| filter.is_allowed(t)).collect();
    if active.is_empty() {
        return String::new();
    }

    // For now, return the full description with a note about which tools are enabled.
    // A more granular per-tool description split can be done later.
    format!(
        "{}\n\nNote: Only the following tools are enabled for this agent: {}",
        TOOLS_DESCRIPTION,
        active.iter().map(|t| **t).collect::<Vec<_>>().join(", ")
    )
}

/// Build skills section, filtered by agent config.
fn build_skills_section(filter: &FilterConfig, env_check: bool) -> String {
    let store = crate::provider::load_store().unwrap_or_default();
    let all_skills = skills::load_all_skills_with_extra(&store.extra_skills_dirs);

    // Start with globally disabled skills
    let disabled = store.disabled_skills.clone();

    // Apply agent-level filtering
    let filtered: Vec<skills::SkillEntry> = all_skills
        .into_iter()
        .filter(|s| filter.is_allowed(&s.name))
        .collect();

    skills::build_skills_prompt(&filtered, &disabled, env_check, &store.skill_env)
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
fn build_runtime_section(model: Option<&str>, provider: Option<&str>, agent_home: Option<&str>) -> String {
    let now = current_date();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
    let os = format!("{} {}", std::env::consts::OS, os_version());
    let arch = std::env::consts::ARCH;
    let hostname = hostname();

    // Working directory: agent home if set, otherwise process cwd
    let working_dir = agent_home
        .map(|h| h.to_string())
        .unwrap_or_else(|| {
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
        lines.push(format!("- Shared directory: {} (shared across all agents — use for cross-agent data exchange)", shared));
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
