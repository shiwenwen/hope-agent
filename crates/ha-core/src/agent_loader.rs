use anyhow::{Context, Result};
use std::path::Path;

use crate::agent_config::{AgentConfig, AgentDefinition, AgentSummary};
use crate::paths;

// ── Constants ────────────────────────────────────────────────────

pub const DEFAULT_AGENT_ID: &str = "default";

/// The Markdown files an agent directory may contain.
const AGENT_MD: &str = "agent.md";
const PERSONA_MD: &str = "persona.md";
const TOOLS_MD: &str = "tools.md";
const MEMORY_MD: &str = "memory.md";

/// 4-file markdown prompt mode files.
const AGENTS_MD: &str = "agents.md";
const IDENTITY_MD: &str = "identity.md";
const SOUL_MD: &str = "soul.md";

// ── Default Agent Template ───────────────────────────────────────

/// Detect system locale code (e.g. "zh", "en", "ja").
pub(crate) fn detect_system_locale() -> String {
    // macOS: check AppleLocale (e.g. "zh_CN", "en_US", "ja_JP")
    if let Ok(output) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
    {
        if let Ok(locale) = String::from_utf8(output.stdout) {
            let locale = locale.trim().to_lowercase();
            // Handle zh_TW / zh_HK specifically
            if locale.starts_with("zh_tw") || locale.starts_with("zh_hk") {
                return "zh-TW".to_string();
            }
            // Return first 2 chars as language code
            if locale.len() >= 2 {
                return locale[..2].to_string();
            }
        }
    }
    // Fallback: LANG env (e.g. "zh_CN.UTF-8")
    for key in &["LANG", "LC_ALL", "LC_MESSAGES"] {
        if let Ok(val) = std::env::var(key) {
            let val = val.to_lowercase();
            if val.starts_with("zh_tw") || val.starts_with("zh_hk") {
                return "zh-TW".to_string();
            }
            if val.len() >= 2 {
                return val[..2].to_string();
            }
        }
    }
    "en".to_string()
}

/// Agent name/description per locale.
struct DefaultMeta {
    name: &'static str,
    description: &'static str,
}

fn default_meta(locale: &str) -> DefaultMeta {
    match locale {
        "zh" => DefaultMeta {
            name: "助手",
            description: "通用 AI 助手",
        },
        "zh-TW" => DefaultMeta {
            name: "助手",
            description: "通用 AI 助手",
        },
        "ja" => DefaultMeta {
            name: "アシスタント",
            description: "汎用 AI アシスタント",
        },
        "ko" => DefaultMeta {
            name: "어시스턴트",
            description: "범용 AI 어시스턴트",
        },
        "es" => DefaultMeta {
            name: "Asistente",
            description: "Asistente de IA de propósito general",
        },
        "pt" => DefaultMeta {
            name: "Assistente",
            description: "Assistente de IA de propósito geral",
        },
        "ru" => DefaultMeta {
            name: "Ассистент",
            description: "Универсальный ИИ-ассистент",
        },
        "ar" => DefaultMeta {
            name: "المساعد",
            description: "مساعد ذكاء اصطناعي متعدد الأغراض",
        },
        "tr" => DefaultMeta {
            name: "Asistan",
            description: "Genel amaçlı yapay zeka asistanı",
        },
        "vi" => DefaultMeta {
            name: "Trợ lý",
            description: "Trợ lý AI đa năng",
        },
        "ms" => DefaultMeta {
            name: "Pembantu",
            description: "Pembantu AI pelbagai guna",
        },
        _ => DefaultMeta {
            name: "Assistant",
            description: "General-purpose AI assistant",
        },
    }
}

/// Agent.md template per locale (embedded at compile time).
pub(crate) fn default_agent_md(locale: &str) -> &'static str {
    match locale {
        "zh" => include_str!("../templates/agent.zh.md"),
        "zh-TW" => include_str!("../templates/agent.zh-TW.md"),
        "ja" => include_str!("../templates/agent.ja.md"),
        "ko" => include_str!("../templates/agent.ko.md"),
        "es" => include_str!("../templates/agent.es.md"),
        "pt" => include_str!("../templates/agent.pt.md"),
        "ru" => include_str!("../templates/agent.ru.md"),
        "ar" => include_str!("../templates/agent.ar.md"),
        "tr" => include_str!("../templates/agent.tr.md"),
        "vi" => include_str!("../templates/agent.vi.md"),
        "ms" => include_str!("../templates/agent.ms.md"),
        _ => include_str!("../templates/agent.en.md"),
    }
}

/// Persona.md template per locale (embedded at compile time).
fn default_persona_md(locale: &str) -> &'static str {
    match locale {
        "zh" => include_str!("../templates/persona.zh.md"),
        "zh-TW" => include_str!("../templates/persona.zh-TW.md"),
        "ja" => include_str!("../templates/persona.ja.md"),
        "ko" => include_str!("../templates/persona.ko.md"),
        "es" => include_str!("../templates/persona.es.md"),
        "pt" => include_str!("../templates/persona.pt.md"),
        "ru" => include_str!("../templates/persona.ru.md"),
        "ar" => include_str!("../templates/persona.ar.md"),
        "tr" => include_str!("../templates/persona.tr.md"),
        "vi" => include_str!("../templates/persona.vi.md"),
        "ms" => include_str!("../templates/persona.ms.md"),
        _ => include_str!("../templates/persona.en.md"),
    }
}

/// 4-file mode templates (English only, no i18n).
fn openclaw_template(name: &str) -> Option<&'static str> {
    match name {
        "openclaw_agents" => Some(include_str!("../templates/openclaw_agents.md")),
        "openclaw_identity" => Some(include_str!("../templates/openclaw_identity.md")),
        "openclaw_soul" => Some(include_str!("../templates/openclaw_soul.md")),
        "openclaw_tools" => Some(include_str!("../templates/openclaw_tools.md")),
        _ => None,
    }
}

/// Get a template by name and locale. Called from frontend.
/// `name`: "agent", "persona", or "openclaw_agents"/"openclaw_identity"/"openclaw_soul"/"openclaw_tools"
/// `locale`: language code like "zh", "en", "ja" etc. (ignored for openclaw templates)
pub fn get_template(name: &str, locale: &str) -> Option<String> {
    // 4-file mode templates (locale-independent)
    if let Some(tpl) = openclaw_template(name) {
        return Some(tpl.to_string());
    }
    match name {
        "agent" => Some(default_agent_md(locale).to_string()),
        "persona" => Some(default_persona_md(locale).to_string()),
        _ => None,
    }
}

fn default_agent_json(locale: &str) -> AgentConfig {
    let meta = default_meta(locale);
    AgentConfig {
        name: meta.name.to_string(),
        description: Some(meta.description.to_string()),
        emoji: Some("🤖".to_string()),
        ..AgentConfig::default()
    }
}

// ── Ensure Default Agent ─────────────────────────────────────────

/// Create the default agent directory and files if they don't exist.
/// Called on app startup.
pub fn ensure_default_agent() -> Result<()> {
    let dir = paths::agent_dir(DEFAULT_AGENT_ID)?;
    let config_path = dir.join("agent.json");

    if config_path.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(&dir)?;

    let locale = detect_system_locale();

    // Write agent.json
    let config = default_agent_json(&locale);
    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, json)?;

    // Write agent.md
    std::fs::write(dir.join(AGENT_MD), default_agent_md(&locale))?;

    Ok(())
}

// ── Load Agent ───────────────────────────────────────────────────

/// Load a complete AgentDefinition from ~/.hope-agent/agents/{id}/
pub fn load_agent(id: &str) -> Result<AgentDefinition> {
    let dir = paths::agent_dir(id)?;
    if !dir.exists() {
        anyhow::bail!("Agent '{}' not found at {}", id, dir.display());
    }

    // Load agent.json (required)
    let config_path = dir.join("agent.json");
    let config: AgentConfig = if config_path.exists() {
        let data = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        serde_json::from_str(&data)
            .with_context(|| format!("Failed to parse {}", config_path.display()))?
    } else {
        AgentConfig::default()
    };

    // Load optional markdown files
    let agent_md = read_optional_md(&dir, AGENT_MD)?;
    let persona = read_optional_md(&dir, PERSONA_MD)?;
    let tools_guide = read_optional_md(&dir, TOOLS_MD)?;
    let memory_md = read_optional_md(&dir, MEMORY_MD)?;

    // Load the 4-file markdown prompt set when openclaw mode is on.
    // In non-openclaw mode we still read SOUL.md when the persona authoring
    // surface is switched to SoulMd, so the same physical `soul.md` file is
    // shared between both surfaces (users keep a single artifact to edit).
    let (agents_md, identity_md, soul_md) = if config.openclaw_mode {
        (
            read_optional_md(&dir, AGENTS_MD)?,
            read_optional_md(&dir, IDENTITY_MD)?,
            read_optional_md(&dir, SOUL_MD)?,
        )
    } else if matches!(
        config.personality.mode,
        crate::agent_config::PersonaMode::SoulMd
    ) {
        (None, None, read_optional_md(&dir, SOUL_MD)?)
    } else {
        (None, None, None)
    };

    // Load global memory.md from ~/.hope-agent/memory.md
    let global_memory_md = {
        let global_path = paths::root_dir()?.join(MEMORY_MD);
        if global_path.exists() {
            Some(
                std::fs::read_to_string(&global_path)
                    .with_context(|| format!("Failed to read {}", global_path.display()))?,
            )
        } else {
            None
        }
    };

    // Ensure agent home directory exists
    if let Ok(home) = paths::agent_home_dir(id) {
        let _ = std::fs::create_dir_all(&home);
    }

    Ok(AgentDefinition {
        id: id.to_string(),
        dir,
        config,
        agent_md,
        persona,
        tools_guide,
        agents_md,
        identity_md,
        soul_md,
        global_memory_md,
        memory_md,
    })
}

/// Read a markdown file if it exists, return None only if file is missing.
/// Returns Some("") for empty files so the frontend can distinguish
/// "never created" (None → fill template) from "user cleared it" (Some("") → keep empty).
fn read_optional_md(dir: &Path, filename: &str) -> Result<Option<String>> {
    let path = dir.join(filename);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    Ok(Some(content))
}

// ── List Agents ──────────────────────────────────────────────────

/// List all available agents from ~/.hope-agent/agents/
pub fn list_agents() -> Result<Vec<AgentSummary>> {
    let agents_dir = paths::agents_dir()?;
    if !agents_dir.exists() {
        return Ok(Vec::new());
    }

    let mut summaries = Vec::new();

    for entry in std::fs::read_dir(&agents_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let id = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        // Try loading the config, skip if invalid
        let config_path = path.join("agent.json");
        let config: AgentConfig = if config_path.exists() {
            match std::fs::read_to_string(&config_path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
            {
                Some(c) => c,
                None => continue,
            }
        } else {
            AgentConfig::default()
        };

        // Count memories for this agent
        let memory_count = crate::get_memory_backend()
            .and_then(|b| {
                b.count(Some(&crate::memory::MemoryScope::Agent { id: id.clone() }))
                    .ok()
            })
            .unwrap_or(0);

        summaries.push(AgentSummary {
            id,
            name: config.name,
            description: config.description,
            emoji: config.emoji,
            avatar: config.avatar,
            has_agent_md: path.join(AGENT_MD).exists(),
            has_persona: path.join(PERSONA_MD).exists(),
            has_tools_guide: path.join(TOOLS_MD).exists(),
            has_memory_md: path.join(MEMORY_MD).exists(),
            memory_count,
            notify_on_complete: config.notify_on_complete,
        });
    }

    // Sort: "default" first, then alphabetical
    summaries.sort_by(|a, b| {
        let a_default = a.id == DEFAULT_AGENT_ID;
        let b_default = b.id == DEFAULT_AGENT_ID;
        b_default.cmp(&a_default).then(a.id.cmp(&b.id))
    });

    Ok(summaries)
}

// ── Save Agent Config ────────────────────────────────────────────

/// Save agent.json for the given agent ID. Creates the directory if needed.
pub fn save_agent_config(id: &str, config: &AgentConfig) -> Result<()> {
    let dir = paths::agent_dir(id)?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("agent.json");
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, json)?;
    Ok(())
}

// ── Save Agent Markdown ──────────────────────────────────────────

/// Save a markdown file for the given agent.
pub fn save_agent_markdown(id: &str, file: &str, content: &str) -> Result<()> {
    // Validate filename to prevent path traversal
    match file {
        AGENT_MD | PERSONA_MD | TOOLS_MD | AGENTS_MD | IDENTITY_MD | SOUL_MD => {}
        _ => anyhow::bail!("Invalid agent markdown file: {}", file),
    }

    let dir = paths::agent_dir(id)?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file);
    std::fs::write(&path, content)?;
    Ok(())
}

// ── Get Agent Markdown ───────────────────────────────────────────

/// Read a markdown file for the given agent.
pub fn get_agent_markdown(id: &str, file: &str) -> Result<Option<String>> {
    match file {
        AGENT_MD | PERSONA_MD | TOOLS_MD | AGENTS_MD | IDENTITY_MD | SOUL_MD => {}
        _ => anyhow::bail!("Invalid agent markdown file: {}", file),
    }
    let dir = paths::agent_dir(id)?;
    read_optional_md(&dir, file)
}

// ── Persona → SOUL.md template ───────────────────────────────────

/// Render the agent's structured `PersonalityConfig` into a SOUL.md markdown
/// template. Used by the UI when the user switches the persona authoring
/// surface to SoulMd for the first time — their existing structured fields
/// are converted into a markdown draft instead of starting from an empty
/// editor. The returned string is never written to disk by this function;
/// the caller decides whether to persist it (usually after the user edits).
pub fn render_persona_to_soul_md(id: &str) -> Result<String> {
    let def = load_agent(id)?;
    let cfg = &def.config;
    let p = &cfg.personality;
    let mut out = String::new();

    out.push_str(&format!("# {} — Who You Are\n", cfg.name));

    if let Some(role) = p.role.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!("\n## Role\n\n{}\n", role));
    }
    if let Some(vibe) = p.vibe.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!("\n## Vibe\n\n{}\n", vibe));
    }
    if let Some(tone) = p.tone.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!("\n## Tone\n\n{}\n", tone));
    }
    if !p.traits.is_empty() {
        out.push_str("\n## Traits\n\n");
        for tr in &p.traits {
            if !tr.trim().is_empty() {
                out.push_str(&format!("- {}\n", tr));
            }
        }
    }
    if !p.principles.is_empty() {
        out.push_str("\n## Principles\n\n");
        for pr in &p.principles {
            if !pr.trim().is_empty() {
                out.push_str(&format!("- {}\n", pr));
            }
        }
    }
    if let Some(b) = p
        .boundaries
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str(&format!("\n## Boundaries\n\n{}\n", b));
    }
    if let Some(q) = p.quirks.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!("\n## Quirks\n\n{}\n", q));
    }
    if let Some(cs) = p
        .communication_style
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str(&format!("\n## Communication Style\n\n{}\n", cs));
    }

    // Leave a gentle nudge when nothing was filled in so the editor isn't
    // just a bare title. Users can delete it once they start writing.
    if !out.contains("##") {
        out.push_str(
            "\n_Describe your persona here: role, tone, values, boundaries, \
             and any quirks that make you distinctive._\n",
        );
    }

    Ok(out)
}

// ── Delete Agent ─────────────────────────────────────────────────

/// Delete an agent directory. Refuses to delete "default".
pub fn delete_agent(id: &str) -> Result<()> {
    if id == DEFAULT_AGENT_ID {
        anyhow::bail!("Cannot delete the default agent");
    }
    let dir = paths::agent_dir(id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}
