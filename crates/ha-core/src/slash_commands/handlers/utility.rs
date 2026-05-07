use crate::config::AppConfig;
use crate::provider;
use crate::session::{MessageRole, SessionDB};
use crate::slash_commands::registry;
use crate::slash_commands::truncate_description;
use crate::slash_commands::types::{
    CommandAction, CommandCategory, CommandResult, SlashCommandDef,
};
use std::sync::Arc;

/// /help — Show all available commands.
///
/// Renders one section per `CommandCategory` (using `description_en()` for the
/// label) plus a `Skills` section. Inside an IM-channel session, commands in
/// `IM_DISABLED_COMMANDS` are filtered out and a footer call-out explains the
/// desktop-only ones.
pub fn handle_help(session_id: Option<&str>) -> CommandResult {
    let in_im_channel = is_session_in_im_channel(session_id);

    let mut commands: Vec<SlashCommandDef> = registry::all_commands();
    if in_im_channel {
        commands.retain(|c| !registry::is_im_disabled(&c.name));
    }

    let cfg = crate::config::cached_config();
    let skills = crate::skills::get_invocable_skills(&cfg.extra_skills_dirs, &cfg.disabled_skills);
    let resolved_skills = crate::slash_commands::resolve_skill_command_names(
        &skills,
        crate::slash_commands::builtin_command_names(),
    );
    drop(cfg);

    let mut lines: Vec<String> = Vec::new();
    lines.push("**Available Commands**".to_string());
    lines.push(String::new());

    // Category order matches the GUI menu (`CATEGORY_ORDER` in
    // `slash-commands/types.ts`) so on-screen and `/help` orderings agree.
    let categories: &[(CommandCategory, &str)] = &[
        (CommandCategory::Session, "Session"),
        (CommandCategory::Model, "Model"),
        (CommandCategory::Memory, "Memory"),
        (CommandCategory::Agent, "Agent"),
        (CommandCategory::Utility, "Utility"),
    ];

    for (cat, label) in categories {
        let cmds: Vec<&SlashCommandDef> = commands.iter().filter(|c| &c.category == cat).collect();
        if cmds.is_empty() {
            continue;
        }
        lines.push(format!("**{}**", label));
        for c in cmds {
            lines.push(format_help_row(c));
        }
        lines.push(String::new());
    }

    if !resolved_skills.is_empty() {
        lines.push(format!("**Skills** ({})", resolved_skills.len()));
        const MAX_SKILLS_INLINE: usize = 20;
        for entry in resolved_skills.iter().take(MAX_SKILLS_INLINE) {
            let desc = truncate_description(&entry.skill.description, 80);
            lines.push(format!("- `/{}` — {}", entry.typed_name, desc));
        }
        if resolved_skills.len() > MAX_SKILLS_INLINE {
            lines.push(format!(
                "- _… and {} more — open the slash menu to browse all_",
                resolved_skills.len() - MAX_SKILLS_INLINE
            ));
        }
        lines.push(String::new());
    }

    if in_im_channel {
        let disabled: Vec<String> = registry::IM_DISABLED_COMMANDS
            .iter()
            .map(|n| format!("`/{}`", n))
            .collect();
        lines.push(format!(
            "_IM channels can't run {} — use the desktop or web app for those._",
            disabled.join(", ")
        ));
    } else {
        lines.push("_Tip: type `/` to open the inline command menu, or click a row above to autofill arguments._".into());
    }

    CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    }
}

/// Resolve whether `session_id` belongs to an IM-channel session. Returns
/// `false` (with a `app_warn!`) on transient SessionDB errors so `/help`
/// always renders something — but a real DB failure is still logged for
/// post-hoc debugging rather than hidden behind an Option-chain.
fn is_session_in_im_channel(session_id: Option<&str>) -> bool {
    let Some(sid) = session_id else {
        return false;
    };
    let Ok(db) = crate::require_session_db() else {
        return false;
    };
    match db.get_session(sid) {
        Ok(Some(meta)) => meta.channel_info.is_some(),
        Ok(None) => false,
        Err(e) => {
            crate::app_warn!(
                "slash_cmd",
                "help",
                "Failed to read session {} for IM-context detection: {}",
                sid,
                e
            );
            false
        }
    }
}

/// Render a single help row: `` `/cmd <args>` — description``. Uses fixed
/// `arg_options` for the inline hint when available (e.g.
/// `/thinking <off|low|medium|high|xhigh>`), otherwise falls back to
/// `arg_placeholder`. `description_en()` is the same source IM channels use
/// for their menu sync, so `/help` and Telegram / Discord menus stay in lockstep.
fn format_help_row(c: &SlashCommandDef) -> String {
    let arg_hint = match (&c.arg_options, c.arg_placeholder.as_deref()) {
        (Some(opts), _) if !opts.is_empty() => {
            let joined = opts.join("|");
            if c.args_optional {
                format!(" [{}]", joined)
            } else {
                format!(" <{}>", joined)
            }
        }
        (_, Some(p)) => format!(" {}", p),
        _ => String::new(),
    };
    format!("- `/{}{}` — {}", c.name, arg_hint, c.description_en())
}

/// /status — Show session status. Aligned with the GUI session-status popover
/// (see `ChatTitleBar.tsx`): version, model + auth type, context window usage,
/// per-turn cache stats, agent, session title + id, message count, permission
/// mode, thinking effort, last-updated relative time, project, IM attaches.
pub async fn handle_status(
    session_db: &Arc<SessionDB>,
    store: &AppConfig,
    session_id: Option<&str>,
    agent_id: &str,
) -> Result<CommandResult, String> {
    let mut lines = vec!["**Session Status**\n".to_string()];

    lines.push(format!("- **Hope Agent**: v{}", env!("CARGO_PKG_VERSION")));

    let active_model_full = store.active_model.as_ref().and_then(|active| {
        provider::build_available_models(&store.providers)
            .into_iter()
            .find(|m| m.provider_id == active.provider_id && m.model_id == active.model_id)
    });

    if let Some(ref active) = store.active_model {
        let (name, auth_label) = if let Some(model) = active_model_full.as_ref() {
            (
                format!("{} / {}", model.provider_name, model.model_name),
                auth_label_for_api_type(&model.api_type),
            )
        } else {
            (
                format!("{} / {}", active.provider_id, active.model_id),
                "api-key",
            )
        };
        lines.push(format!("- **Model**: {} ({})", name, auth_label));
    } else {
        lines.push("- **Model**: not set".into());
    }

    lines.push(format!("- **Agent**: `{}`", agent_id));

    if let Some(sid) = session_id {
        let meta = session_db.get_session(sid).ok().flatten();

        if let Some(title) = meta
            .as_ref()
            .and_then(|m| m.title.as_deref())
            .filter(|s| !s.trim().is_empty())
        {
            lines.push(format!("- **Title**: {}", truncate_description(title, 200)));
        }
        lines.push(format!("- **Session ID**: `{}`", sid));

        if let Ok((user_count, assistant_count)) = session_db.count_user_assistant_messages(sid) {
            lines.push(format!(
                "- **Messages**: {} user, {} assistant",
                user_count, assistant_count
            ));
        }

        let mode = session_db
            .get_session_permission_mode(sid)
            .ok()
            .flatten()
            .unwrap_or(crate::permission::SessionMode::Default);
        lines.push(format!("- **Permission Mode**: `{}`", mode.as_str()));

        let effort = resolve_status_reasoning_effort(meta.as_ref()).await;
        lines.push(format!("- **Thinking**: {}", effort));

        if let Ok(Some(tokens)) = session_db.get_session_last_assistant_token_row(sid) {
            let context_window = active_model_full
                .as_ref()
                .map(|m| m.context_window)
                .or_else(|| context_window_for_model_id(tokens.model.as_deref()))
                .unwrap_or(0);
            let used = tokens
                .tokens_in_last
                .or(tokens.tokens_in)
                .unwrap_or(0)
                .max(0) as u64;
            lines.push(format_context_usage_line(used, context_window));

            let creation = tokens.tokens_cache_creation.unwrap_or(0).max(0) as u64;
            let read = tokens.tokens_cache_read.unwrap_or(0).max(0) as u64;
            if creation > 0 || read > 0 {
                lines.push(format!(
                    "- **Cache (last round)**: write {} · hit {}",
                    format_token_count(creation),
                    format_token_count(read)
                ));
            }
        }

        if let Some(updated_at) = meta.as_ref().map(|m| m.updated_at.as_str()) {
            if let Some(rel) = format_duration_since(updated_at) {
                lines.push(format!("- **Updated**: {}", rel));
            }
        }

        if let Some(meta_ref) = meta.as_ref() {
            if let Some(project_lines) = render_project_section(meta_ref) {
                lines.push(String::new());
                lines.extend(project_lines);
            }
        }
        if let Some(channel_lines) = render_attached_channels_section(sid) {
            lines.push(String::new());
            lines.extend(channel_lines);
        }
    } else {
        lines.push("- **Session**: none (new chat)".into());
    }

    Ok(CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// Auth descriptor for `/status`'s Model line — Codex provider runs on
/// ChatGPT OAuth; everything else is API key.
fn auth_label_for_api_type(api_type: &crate::provider::ApiType) -> &'static str {
    match api_type {
        crate::provider::ApiType::Codex => "oauth",
        _ => "api-key",
    }
}

/// Resolve the effective thinking effort to render in `/status`. Two-layer
/// fallback: persisted `sessions.reasoning_effort` (UI / `/think` writes here)
/// → live global `REASONING_EFFORT` cell (already maps `"none"` → `None`)
/// → `"medium"` default. Mirrors the GUI popover's "Thinking" line.
async fn resolve_status_reasoning_effort(meta: Option<&crate::session::SessionMeta>) -> String {
    let from_session = meta
        .and_then(|m| m.reasoning_effort.clone())
        .filter(|s| !s.trim().is_empty() && s.trim() != "none");
    let effort = match from_session {
        Some(s) => Some(s),
        None => crate::agent::live_reasoning_effort(None).await,
    };
    effort.unwrap_or_else(|| "medium".to_string())
}

/// Best-effort context window lookup when the row's model is no longer in the
/// active provider list (model was removed, or row was written by a different
/// provider). Returns `None` to let the renderer show only the absolute usage.
fn context_window_for_model_id(model_id: Option<&str>) -> Option<u32> {
    let id = model_id?.to_ascii_lowercase();
    let cfg = crate::config::cached_config();
    for provider in &cfg.providers {
        for model in &provider.models {
            if model.id.eq_ignore_ascii_case(&id) {
                return Some(model.context_window);
            }
        }
    }
    None
}

fn format_context_usage_line(used: u64, context_window: u32) -> String {
    if context_window == 0 {
        return format!("- **Context**: {}", format_token_count(used));
    }
    let pct = ((used as f64 / context_window as f64) * 100.0).round() as u64;
    format!(
        "- **Context**: {} / {} ({}%)",
        format_token_count(used),
        format_token_count(context_window as u64),
        pct
    )
}

fn format_token_count(tokens: u64) -> String {
    if tokens >= 1_000 {
        format!("{}k", (tokens as f64 / 1_000.0).round() as u64)
    } else {
        tokens.to_string()
    }
}

/// Format an RFC3339-ish timestamp as a coarse "2h ago" / "just now" relative
/// string. Returns `None` when the timestamp is unparseable so the caller can
/// drop the row entirely rather than render a broken value.
///
/// `sessions.updated_at` is always written via `Utc::now().to_rfc3339()`
/// (see `SessionDB`), so RFC3339 is the only format we need to handle.
fn format_duration_since(ts: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(ts)
        .ok()?
        .with_timezone(&chrono::Utc);
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(parsed);
    let secs = delta.num_seconds();
    if secs < 0 {
        return Some("just now".to_string());
    }
    if secs < 60 {
        return Some("just now".to_string());
    }
    let mins = secs / 60;
    if mins < 60 {
        return Some(format!("{}m ago", mins));
    }
    let hours = mins / 60;
    if hours < 24 {
        return Some(format!("{}h ago", hours));
    }
    let days = hours / 24;
    Some(format!("{}d ago", days))
}

fn render_project_section(meta: &crate::session::SessionMeta) -> Option<Vec<String>> {
    let project_id = meta.project_id.as_deref()?;
    let project_db = crate::require_project_db().ok()?;
    let project = project_db.get(project_id).ok().flatten()?;

    let mut lines = vec![
        "**Current Project**".to_string(),
        format!("- **Name**: {}", project.name),
    ];
    if let Some(desc) = project
        .description
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        lines.push(format!(
            "- **Description**: {}",
            truncate_description(desc, 200)
        ));
    }
    if let Some(default_agent) = project.default_agent_id.as_deref() {
        lines.push(format!("- **Default Agent**: `{}`", default_agent));
    }
    if let Some(working_dir) = project.working_dir.as_deref() {
        lines.push(format!("- **Working Directory**: `{}`", working_dir));
    }
    if let Some(instructions) = project
        .instructions
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        lines.push(format!(
            "- **Instructions**: {}",
            truncate_description(instructions, 200)
        ));
    }

    let cfg = crate::config::cached_config();
    let channel_account = meta
        .channel_info
        .as_ref()
        .and_then(|ci| cfg.channels.find_account(&ci.account_id))
        .cloned();
    let (_, source) = crate::agent::resolver::resolve_default_agent_id_with_source(
        Some(&project),
        channel_account.as_ref(),
    );
    lines.push(format!("- **Agent Source**: {}", source.label()));
    Some(lines)
}

/// `/status` IM-attach section: shows the chat currently attached to
/// the session (1:1, so 0 or 1 row). Returns `None` when no attach
/// exists so the caller can skip the empty section header.
fn render_attached_channels_section(sid: &str) -> Option<Vec<String>> {
    let channel_db = crate::globals::get_channel_db()?;
    let attach = channel_db.get_conversation_by_session(sid).ok().flatten()?;
    Some(vec![
        "**Attached IM Channel**".to_string(),
        super::format_attached_channel_line(&attach, true),
    ])
}

/// /export — Export conversation as Markdown.
pub fn handle_export(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session to export")?;
    let messages = session_db
        .load_session_messages(sid)
        .map_err(|e| e.to_string())?;

    if messages.is_empty() {
        return Err("No messages to export".into());
    }

    let session_meta = session_db.get_session(sid).map_err(|e| e.to_string())?;
    let title = session_meta
        .and_then(|m| m.title)
        .unwrap_or_else(|| "Untitled".to_string());

    let mut md = format!("# {}\n\n", title);
    for msg in &messages {
        match msg.role {
            MessageRole::User => {
                md.push_str(&format!("## User\n\n{}\n\n", msg.content));
            }
            MessageRole::Assistant => {
                md.push_str(&format!("## Assistant\n\n{}\n\n", msg.content));
            }
            _ => {}
        }
    }

    let filename = format!("{}.md", sanitize_filename(&title));

    Ok(CommandResult {
        content: format!("Exported {} messages.", messages.len()),
        action: Some(CommandAction::ExportFile {
            content: md,
            filename,
        }),
    })
}

/// /usage — Show token usage for current session.
pub fn handle_usage(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session")?;
    let messages = session_db
        .load_session_messages(sid)
        .map_err(|e| e.to_string())?;

    let mut total_in: i64 = 0;
    let mut total_out: i64 = 0;
    let mut turns = 0;

    for msg in &messages {
        if msg.role == MessageRole::Assistant {
            turns += 1;
            total_in += msg.tokens_in.unwrap_or(0);
            total_out += msg.tokens_out.unwrap_or(0);
        }
    }

    let content = format!(
        "**Token Usage**\n\n- **Input tokens**: {}\n- **Output tokens**: {}\n- **Total**: {}\n- **Turns**: {}",
        total_in,
        total_out,
        total_in + total_out,
        turns,
    );

    Ok(CommandResult {
        content,
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /permission <default|smart|yolo> — Switch the session permission mode.
/// Use `/status` to view the current mode.
pub fn handle_permission(args: &str) -> Result<CommandResult, String> {
    let mode_arg = args.trim().to_lowercase();
    let resolved = match mode_arg.as_str() {
        "default" => crate::permission::SessionMode::Default,
        "smart" => crate::permission::SessionMode::Smart,
        "yolo" => crate::permission::SessionMode::Yolo,
        _ => {
            return Err(format!(
                "Invalid permission mode: `{}`. Valid: default, smart, yolo",
                mode_arg
            ));
        }
    };

    Ok(CommandResult {
        content: format!("Permission mode set to **{}**.", resolved.as_str()),
        action: Some(CommandAction::SetToolPermission {
            mode: resolved.as_str().to_string(),
        }),
    })
}

/// /search <query> — Pass through to LLM as a search request.
pub fn handle_search(args: &str) -> Result<CommandResult, String> {
    let query = args.trim();
    if query.is_empty() {
        return Err("Usage: /search <query>".into());
    }
    Ok(CommandResult {
        content: String::new(),
        action: Some(CommandAction::PassThrough {
            message: format!("Please search the web for: {}", query),
        }),
    })
}

/// /imreply [split|final|preview] — Show or set the IM reply mode for the
/// current channel account. Three modes, see [`crate::channel::ImReplyMode`]:
///
/// - **`split`** (default): each round (narration + media) delivered in time
///   order as independent messages. Streaming channels still get a typewriter
///   effect *per round*, just not "one growing message".
/// - **`final`**: only the last-round narration + all media in one burst.
///   No streaming preview.
/// - **`preview`**: streaming channels render the full merged response in a
///   single growing preview message (Telegram edit / Feishu cardkit / Telegram
///   DM draft); non-streaming channels degrade to `final`.
///
/// Persisted to `ChannelAccountConfig.settings.imReplyMode` via [`mutate_config`].
pub async fn handle_imreply(session_id: Option<&str>, args: &str) -> Result<CommandResult, String> {
    let Some(sid) = session_id else {
        return Err("/imreply only works inside an IM channel session.".into());
    };
    let session_db = crate::require_session_db().map_err(|e| e.to_string())?;
    let channel_info = session_db
        .get_session(sid)
        .map_err(|e| e.to_string())?
        .and_then(|m| m.channel_info)
        .ok_or_else(|| "/imreply only works inside an IM channel session.".to_string())?;

    let cfg = crate::config::cached_config();
    let account = cfg
        .channels
        .accounts
        .iter()
        .find(|a| a.id == channel_info.account_id)
        .ok_or_else(|| {
            format!(
                "Channel account `{}` not found in config",
                channel_info.account_id
            )
        })?;
    let current = account.im_reply_mode();
    drop(cfg);

    let arg = args.trim();
    if arg.is_empty() {
        return Ok(CommandResult {
            content: format!(
                "**IM reply mode**: `{}`\n\n- `split` — each round in time order, separate messages (default; recommended)\n- `final` — only the last-round answer + all media at the end\n- `preview` — single growing preview message (streaming channels only; degrades to `final` elsewhere)\n\nUsage: `/imreply split` · `/imreply final` · `/imreply preview`",
                current.as_str()
            ),
            action: Some(CommandAction::DisplayOnly),
        });
    }

    let mode = crate::channel::ImReplyMode::parse(arg)
        .ok_or_else(|| format!("Invalid mode: `{}`. Valid: split, final, preview", arg))?;

    let account_id = channel_info.account_id.clone();
    let mode_str = mode.as_str();
    crate::config::mutate_config(("channel.imReplyMode", "slash:/imreply"), |cfg| {
        match cfg
            .channels
            .accounts
            .iter_mut()
            .find(|a| a.id == account_id)
        {
            Some(acc) => {
                acc.set_im_reply_mode(mode);
                Ok(())
            }
            None => Err(anyhow::anyhow!(
                "Channel account `{}` not found in config",
                account_id
            )),
        }
    })
    .map_err(|e| e.to_string())?;

    Ok(CommandResult {
        content: format!(
            "IM reply mode set to **{}** for this channel account.",
            mode_str
        ),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// `/reason` (alias `/reasoning`) — IM-only. Toggle whether the model's
/// thinking/reasoning content is included in outbound IM messages for the
/// current channel account. Default off — reasoning stays out of IM.
///
/// Persisted to `ChannelAccountConfig.settings.showThinking` via
/// [`mutate_config`]. When enabled, the round accumulator wraps reasoning
/// in a markdown blockquote (`> 💭 **Thinking**`) before the round's reply
/// text.
pub async fn handle_reason(session_id: Option<&str>, args: &str) -> Result<CommandResult, String> {
    let Some(sid) = session_id else {
        return Err("/reason only works inside an IM channel session.".into());
    };
    let session_db = crate::require_session_db().map_err(|e| e.to_string())?;
    let channel_info = session_db
        .get_session(sid)
        .map_err(|e| e.to_string())?
        .and_then(|m| m.channel_info)
        .ok_or_else(|| "/reason only works inside an IM channel session.".to_string())?;

    let cfg = crate::config::cached_config();
    let account = cfg
        .channels
        .accounts
        .iter()
        .find(|a| a.id == channel_info.account_id)
        .ok_or_else(|| {
            format!(
                "Channel account `{}` not found in config",
                channel_info.account_id
            )
        })?;
    let current = account.show_thinking();
    drop(cfg);

    let arg = args.trim();
    if arg.is_empty() {
        let current_label = if current { "on" } else { "off" };
        return Ok(CommandResult {
            content: format!(
                "**Show thinking in IM**: `{}`\n\n- `on` — render the model's reasoning as a quoted block before each round's reply\n- `off` — drop reasoning from IM messages (default)\n\nUsage: `/reason on` · `/reason off`",
                current_label
            ),
            action: Some(CommandAction::DisplayOnly),
        });
    }

    let value = match arg.to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => return Err(format!("Invalid value: `{}`. Valid: on, off", arg)),
    };

    let account_id = channel_info.account_id.clone();
    crate::config::mutate_config(("channel.showThinking", "slash:/reason"), |cfg| {
        match cfg
            .channels
            .accounts
            .iter_mut()
            .find(|a| a.id == account_id)
        {
            Some(acc) => {
                acc.set_show_thinking(value);
                Ok(())
            }
            None => Err(anyhow::anyhow!(
                "Channel account `{}` not found in config",
                account_id
            )),
        }
    })
    .map_err(|e| e.to_string())?;

    Ok(CommandResult {
        content: format!(
            "Show thinking set to **{}** for this channel account.",
            if value { "on" } else { "off" }
        ),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /prompts — Open the system prompt viewer.
pub fn handle_prompts() -> CommandResult {
    CommandResult {
        content: String::new(),
        action: Some(CommandAction::ViewSystemPrompt),
    }
}

/// Simple filename sanitization.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_modes_emit_set_action() {
        for (input, expected) in [
            ("default", "default"),
            ("smart", "smart"),
            ("yolo", "yolo"),
            // case-insensitive — handler lowercases args
            ("YOLO", "yolo"),
            ("  smart  ", "smart"),
        ] {
            let res = handle_permission(input).expect("ok");
            match res.action {
                Some(CommandAction::SetToolPermission { ref mode }) => {
                    assert_eq!(mode, expected, "input {:?}", input);
                }
                other => panic!("unexpected action for {:?}: {:?}", input, other),
            }
            assert!(res.content.contains(&format!("**{}**", expected)));
        }
    }

    #[test]
    fn format_token_count_collapses_thousands() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(999), "999");
        assert_eq!(format_token_count(1_000), "1k");
        assert_eq!(format_token_count(1_499), "1k");
        assert_eq!(format_token_count(1_500), "2k");
        assert_eq!(format_token_count(200_000), "200k");
    }

    #[test]
    fn format_context_usage_line_handles_unknown_window() {
        let s = format_context_usage_line(1_234, 0);
        assert_eq!(s, "- **Context**: 1k");
        let s = format_context_usage_line(50_000, 200_000);
        assert_eq!(s, "- **Context**: 50k / 200k (25%)");
    }

    #[test]
    fn format_duration_since_rolls_buckets() {
        let now = chrono::Utc::now();
        let just = now - chrono::Duration::seconds(5);
        assert_eq!(
            format_duration_since(&just.to_rfc3339()).as_deref(),
            Some("just now")
        );
        let mins_ago = now - chrono::Duration::minutes(7);
        assert_eq!(
            format_duration_since(&mins_ago.to_rfc3339()).as_deref(),
            Some("7m ago")
        );
        let hours_ago = now - chrono::Duration::hours(3);
        assert_eq!(
            format_duration_since(&hours_ago.to_rfc3339()).as_deref(),
            Some("3h ago")
        );
        let days_ago = now - chrono::Duration::days(2);
        assert_eq!(
            format_duration_since(&days_ago.to_rfc3339()).as_deref(),
            Some("2d ago")
        );
        assert!(format_duration_since("not-a-time").is_none());
    }

    #[test]
    fn auth_label_for_api_type_branches() {
        assert_eq!(
            auth_label_for_api_type(&crate::provider::ApiType::Codex),
            "oauth"
        );
        assert_eq!(
            auth_label_for_api_type(&crate::provider::ApiType::Anthropic),
            "api-key"
        );
        assert_eq!(
            auth_label_for_api_type(&crate::provider::ApiType::OpenaiChat),
            "api-key"
        );
    }

    /// `SessionDB::open` doesn't create the `channel_conversations` table —
    /// that lives in `ChannelDB::migrate` — but `SESSION_META_SELECT` LEFT
    /// JOINs it. Mirror the prod schema for the `/status` test fixture so
    /// `get_session` doesn't error out on a missing table.
    fn ensure_channel_conversations_table(db: &SessionDB) {
        let conn = db.conn.lock().expect("lock");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS channel_conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                chat_id TEXT NOT NULL,
                thread_id TEXT,
                session_id TEXT NOT NULL,
                sender_id TEXT,
                sender_name TEXT,
                chat_type TEXT NOT NULL DEFAULT 'dm',
                is_primary INTEGER NOT NULL DEFAULT 1,
                source TEXT NOT NULL DEFAULT 'inbound',
                attached_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );",
        )
        .expect("create channel_conversations");
    }

    #[tokio::test]
    async fn handle_status_renders_full_session_panel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.db");
        let db = Arc::new(SessionDB::open(&path).expect("open"));
        ensure_channel_conversations_table(&db);
        let meta = db.create_session("default").expect("create");
        let sid = meta.id.clone();
        db.update_session_title(&sid, "Glittery island recap")
            .expect("title");
        db.update_session_reasoning_effort(&sid, Some("high"))
            .expect("effort");

        let now = chrono::Utc::now().to_rfc3339();
        let assistant = crate::session::NewMessage {
            role: crate::session::MessageRole::Assistant,
            content: "ok".into(),
            timestamp: now.clone(),
            attachments_meta: None,
            model: Some("test-model".into()),
            tokens_in: Some(45_678),
            tokens_out: Some(120),
            reasoning_effort: Some("high".into()),
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: Some(42_000),
            tokens_cache_creation: Some(1_500),
            tokens_cache_read: Some(38_000),
            tool_metadata: None,
            stream_status: None,
            source: Some("desktop".into()),
        };
        db.append_message(&sid, &assistant).expect("append");

        let store = AppConfig::default();
        let result = handle_status(&db, &store, Some(&sid), "default")
            .await
            .expect("ok");
        assert!(result.content.contains("**Hope Agent**: v"));
        assert!(result.content.contains("**Title**: Glittery island recap"));
        assert!(result
            .content
            .contains(&format!("**Session ID**: `{}`", sid)));
        assert!(result.content.contains("**Messages**: 0 user, 1 assistant"));
        assert!(result.content.contains("**Permission Mode**: `default`"));
        assert!(result.content.contains("**Thinking**: high"));
        assert!(result.content.contains("**Context**: 42k"));
        assert!(result.content.contains("**Cache (last round)**:"));
        assert!(result.content.contains("write 2k"));
        assert!(result.content.contains("hit 38k"));
        assert!(result.content.contains("**Updated**:"));
    }

    #[tokio::test]
    async fn handle_status_skips_token_lines_when_no_assistant_msgs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.db");
        let db = Arc::new(SessionDB::open(&path).expect("open"));
        ensure_channel_conversations_table(&db);
        let meta = db.create_session("default").expect("create");
        let sid = meta.id.clone();

        let store = AppConfig::default();
        let result = handle_status(&db, &store, Some(&sid), "default")
            .await
            .expect("ok");
        assert!(result.content.contains("**Hope Agent**: v"));
        assert!(result
            .content
            .contains(&format!("**Session ID**: `{}`", sid)));
        assert!(!result.content.contains("**Context**:"));
        assert!(!result.content.contains("**Cache (last round)**:"));
        assert!(result.content.contains("**Thinking**:"));
    }

    #[test]
    fn rejects_legacy_and_unknown_aliases() {
        for bad in [
            "auto",
            "ask",
            "full",
            "ask_every_time",
            "full_approve",
            "garbage",
            "",
        ] {
            let err = handle_permission(bad).expect_err("should error");
            assert!(
                err.contains("Invalid permission mode") && err.contains("default, smart, yolo"),
                "input {:?}, got {:?}",
                bad,
                err
            );
        }
    }
}
