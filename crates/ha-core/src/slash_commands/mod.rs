pub mod fuzzy;
pub mod handlers;
pub mod parser;
pub mod registry;
pub mod types;

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::skills::SkillEntry;
use types::{
    CommandAction, CommandCategory, CommandResult, ModelPickerItem, ProjectPickerItem,
    SessionPickerItem, SlashCommandDef,
};

/// A user-typed slash command name paired with the originating SkillEntry.
/// `typed_name` may differ from the skill's canonical name when collision
/// resolution added a `_skill` / `_N` suffix.
pub struct ResolvedSkillCommand<'a> {
    pub typed_name: String,
    pub skill: &'a SkillEntry,
}

/// Resolve each skill's user-typed command name against `reserved`.
///
/// Rules: canonical name collides → append `_skill`, then `_2`/`_3`/... until
/// free; alias collides → dropped. Shared by listing and dispatch so the
/// typed name stays in sync with the runtime-matched skill.
pub fn resolve_skill_command_names<'a>(
    skills: &'a [SkillEntry],
    reserved: &HashSet<String>,
) -> Vec<ResolvedSkillCommand<'a>> {
    let mut used: HashSet<String> = reserved.clone();
    let mut out: Vec<ResolvedSkillCommand<'a>> = Vec::with_capacity(skills.len());

    for skill in skills {
        let mut names_iter = skill.all_command_names();
        let canonical = names_iter.next().expect("canonical name always yielded");

        let mut display = if used.contains(&canonical) {
            format!("{}_skill", canonical)
        } else {
            canonical.clone()
        };
        let base = display.clone();
        let mut counter = 2;
        while used.contains(&display) {
            display = format!("{}_{}", base, counter);
            counter += 1;
        }
        used.insert(display.clone());
        out.push(ResolvedSkillCommand {
            typed_name: display,
            skill,
        });

        for alias in names_iter {
            if used.contains(&alias) {
                continue;
            }
            used.insert(alias.clone());
            out.push(ResolvedSkillCommand {
                typed_name: alias,
                skill,
            });
        }
    }

    out
}

/// Silent dispatcher aliases — names accepted by the dispatch match arm
/// in `handlers::mod.rs` but deliberately NOT registered in
/// `registry::all_commands()` (so they don't appear in IM slash menus).
/// They must still be reserved against skill name collisions, otherwise
/// a user-defined `/<alias>` skill would be shadowed silently by the
/// built-in dispatch.
const SILENT_BUILTIN_ALIASES: &[&str] = &["reasoning", "think"];

/// Resolve silent built-in aliases to their canonical command names for
/// metadata lookup paths (arg options, help text, etc.). Dispatch still matches
/// aliases explicitly so the behavior stays obvious at the side-effect boundary.
pub fn canonical_builtin_command_name(name: &str) -> &str {
    match name {
        "reasoning" => "reason",
        "think" => "thinking",
        _ => name,
    }
}

/// Built-in (hardcoded) slash command names — cached since `registry::all_commands()`
/// is compile-time constant. Includes silent dispatcher aliases (see
/// [`SILENT_BUILTIN_ALIASES`]) so the skill collision check in
/// [`resolve_skill_command_names`] sees them too.
pub fn builtin_command_names() -> &'static HashSet<String> {
    static CACHE: OnceLock<HashSet<String>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut names: HashSet<String> = registry::all_commands()
            .into_iter()
            .map(|c| c.name)
            .collect();
        for alias in SILENT_BUILTIN_ALIASES {
            names.insert((*alias).to_string());
        }
        names
    })
}

/// List all available slash commands (for UI menu rendering).
/// Includes both built-in commands and user-invocable skill commands.
pub async fn list_slash_commands() -> Result<Vec<SlashCommandDef>, String> {
    let mut commands = registry::all_commands();

    let store = crate::config::cached_config();
    let skill_entries =
        crate::skills::get_invocable_skills(&store.extra_skills_dirs, &store.disabled_skills);
    let skill_entries = crate::skills::filter_catalog_eligible_skills(
        skill_entries,
        store.skill_env_check,
        &store.skill_env,
    );
    drop(store);

    // Use the cached reserved-name set so silent dispatcher aliases
    // (e.g. `reasoning` for `/reason`) also block skill collisions.
    let reserved = builtin_command_names();
    let resolved = resolve_skill_command_names(&skill_entries, reserved);

    for entry in resolved {
        let skill = entry.skill;
        let arg_placeholder = skill
            .command_arg_placeholder
            .clone()
            .or_else(|| Some("[args]".into()));
        let arg_options = skill.command_arg_options.clone();
        let description_raw = Some(truncate_description(&skill.description, 100));

        commands.push(SlashCommandDef {
            name: entry.typed_name,
            category: CommandCategory::Skill,
            description_key: String::new(),
            has_args: true,
            args_optional: true,
            arg_placeholder,
            arg_options,
            description_raw,
        });
    }

    Ok(commands)
}

/// Execute a slash command.
///
/// - `session_id`: Current session ID (None if no active session)
/// - `agent_id`: Current agent ID
/// - `command_text`: Full text including "/" prefix, e.g. "/model gpt-4o"
pub async fn execute_slash_command(
    session_id: Option<String>,
    agent_id: String,
    command_text: String,
) -> Result<CommandResult, String> {
    let (name, args) = parser::parse(&command_text)?;

    // Allow both built-in commands and dynamic skill commands
    // (skill commands are handled in handlers::dispatch fallback)

    // UserPromptExpansion hook (observation): a slash command ran, matchable on
    // the command name.
    crate::hooks::fire_user_prompt_expansion(
        session_id.as_deref(),
        &agent_id,
        &name,
        &command_text,
    );

    app_info!(
        "slash_cmd",
        "dispatch",
        "Executing /{} args={:?}",
        name,
        args
    );

    let result = handlers::dispatch(session_id.as_deref(), &agent_id, &name, &args).await?;

    if let Some(sid) = session_id.as_deref() {
        if should_persist_slash_history(result.action.as_ref()) {
            match crate::get_session_db() {
                Some(db) => {
                    if let Err(e) = append_slash_history_result_events(
                        &db,
                        sid,
                        &command_text,
                        &result,
                        crate::chat_engine::ChatSource::Desktop,
                    ) {
                        app_warn!(
                            "slash_cmd",
                            "history",
                            "Failed to persist slash command history: {}",
                            e
                        );
                    }
                }
                None => app_warn!(
                    "slash_cmd",
                    "history",
                    "SessionDB unavailable while persisting slash command history"
                ),
            }
        }
    }

    app_info!(
        "slash_cmd",
        "dispatch",
        "/{} completed: action={:?}",
        name,
        result
            .action
            .as_ref()
            .map(|a| format!("{:?}", a).chars().take(50).collect::<String>())
    );

    Ok(result)
}

/// Persist a control slash command to visible transcript without feeding it
/// to the LLM. The command row is styled like a user bubble by the frontend;
/// the result row remains a normal event chip/card.
pub fn append_slash_history_events(
    session_db: &crate::session::SessionDB,
    session_id: &str,
    command_text: &str,
    result_content: Option<&str>,
    source: crate::chat_engine::ChatSource,
) -> anyhow::Result<Vec<i64>> {
    let command_text = command_text.trim();
    if command_text.is_empty() {
        return Ok(Vec::new());
    }

    let mut ids = Vec::with_capacity(2);
    let command_display = slash_history_command_display(command_text);

    let mut command_msg =
        crate::session::NewMessage::event(&command_display.content).with_source(source);
    command_msg.attachments_meta = Some(
        serde_json::json!({
            "slash_command": {
                "kind": "command",
                "command": command_text,
                "displayAs": "user",
                "mode": command_display.mode,
            }
        })
        .to_string(),
    );
    ids.push(session_db.append_message(session_id, &command_msg)?);
    let _ = crate::session::ensure_first_message_title(
        session_db,
        session_id,
        &command_display.content,
        None,
    );

    if let Some(result_content) = result_content.filter(|s| !s.trim().is_empty()) {
        let mut result_msg = crate::session::NewMessage::event(result_content).with_source(source);
        result_msg.attachments_meta = Some(
            serde_json::json!({
                "slash_command": {
                    "kind": "result",
                    "command": command_text,
                }
            })
            .to_string(),
        );
        ids.push(session_db.append_message(session_id, &result_msg)?);
    }

    Ok(ids)
}

struct SlashHistoryCommandDisplay {
    content: String,
    mode: Option<&'static str>,
}

fn slash_history_command_display(command_text: &str) -> SlashHistoryCommandDisplay {
    let trimmed = command_text.trim();
    if let Some(args) = slash_command_args(trimmed, "goal") {
        let content = goal_command_display_text(args)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Goal".to_string());
        return SlashHistoryCommandDisplay {
            content,
            mode: Some("goal"),
        };
    }
    if let Some(args) = slash_command_args(trimmed, "loop") {
        let content = loop_command_display_text(args)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Loop".to_string());
        return SlashHistoryCommandDisplay {
            content,
            mode: Some("loop"),
        };
    }
    SlashHistoryCommandDisplay {
        content: trimmed.to_string(),
        mode: None,
    }
}

fn slash_command_args<'a>(trimmed: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("/{name}");
    let raw_rest = trimmed.strip_prefix(&prefix)?;
    if !raw_rest.is_empty() && !raw_rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(raw_rest.trim())
}

fn goal_command_display_text(args: &str) -> Option<String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Some("Show active goal".to_string());
    }
    match trimmed {
        "status" | "show" => Some("Show active goal".to_string()),
        "pause" => Some("Pause active goal".to_string()),
        "resume" => Some("Resume active goal".to_string()),
        "clear" | "cancel" => Some("Clear active goal".to_string()),
        "evaluate" | "audit" => Some("Evaluate active goal".to_string()),
        "help" => Some("Goal help".to_string()),
        _ => Some(trimmed.to_string()),
    }
}

fn loop_command_display_text(args: &str) -> Option<String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Some("Start self-paced loop".to_string());
    }
    let first = trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match first.as_str() {
        "status" | "list" | "show" => Some("Show loops".to_string()),
        "pause" => Some("Pause loop".to_string()),
        "resume" => Some("Resume loop".to_string()),
        "stop" | "cancel" => Some("Stop loop".to_string()),
        "help" => Some("Loop help".to_string()),
        _ => Some(trimmed.to_string()),
    }
}

fn is_loop_create_slash_command(command_text: &str) -> bool {
    let Some(args) = slash_command_args(command_text.trim(), "loop") else {
        return false;
    };
    if args.trim().is_empty() {
        return false;
    }
    let first = args
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    !matches!(
        first.as_str(),
        "status" | "list" | "show" | "help" | "pause" | "resume" | "stop" | "cancel"
    )
}

/// Persist a full `CommandResult`, including markdown fallbacks for structured
/// actions whose live desktop UI is card/modal based and therefore has empty
/// `content`.
pub fn append_slash_history_result_events(
    session_db: &crate::session::SessionDB,
    session_id: &str,
    command_text: &str,
    result: &CommandResult,
    source: crate::chat_engine::ChatSource,
) -> anyhow::Result<Vec<i64>> {
    let fallback = slash_history_result_content(command_text, result);
    append_slash_history_events(
        session_db,
        session_id,
        command_text,
        fallback.as_deref(),
        source,
    )
}

fn slash_history_result_content(command_text: &str, result: &CommandResult) -> Option<String> {
    if is_loop_create_slash_command(command_text) {
        return None;
    }
    if !result.content.trim().is_empty() {
        return Some(result.content.clone());
    }

    let action = result.action.as_ref()?;
    match action {
        CommandAction::ShowModelPicker {
            models,
            active_provider_id,
            active_model_id,
        } => Some(render_model_picker_history(
            models,
            active_provider_id.as_deref(),
            active_model_id.as_deref(),
        )),
        CommandAction::ShowProjectPicker { projects } => {
            Some(render_project_picker_history(projects))
        }
        CommandAction::ShowSessionPicker { sessions } => {
            Some(render_session_picker_history(sessions))
        }
        CommandAction::EnterPlanMode => Some("Entered plan mode.".into()),
        CommandAction::ExitPlanMode { .. } => Some("Exited plan mode.".into()),
        CommandAction::ApprovePlan { .. } => Some("Plan approved. Starting execution.".into()),
        CommandAction::ShowPlan { plan_content } => {
            Some(format!("**Current Plan**\n\n{}", plan_content))
        }
        CommandAction::SetWorkflowMode { mode } => Some(format!("Workflow Mode set to `{mode}`.")),
        CommandAction::ViewSystemPrompt => Some("Opened system prompt viewer.".into()),
        CommandAction::OpenDashboardTab { tab } => Some(format!("Opened Dashboard tab `{}`.", tab)),
        CommandAction::RecapCard { report_id } => Some(format!(
            "Started recap report `{}`.",
            crate::truncate_utf8(report_id, 8)
        )),
        CommandAction::SkillFork { run_id, skill_name } => Some(format!(
            "Skill **{}** forked to sub-agent (run: {}). Result will be injected when complete.",
            skill_name,
            crate::truncate_utf8(run_id, 8)
        )),
        CommandAction::EnterProject { project_id } => Some(format!(
            "Entering project `{}`.",
            crate::truncate_utf8(project_id, 8)
        )),
        CommandAction::AssignProject { project_id } => Some(format!(
            "Linked this session to project `{}`.",
            crate::truncate_utf8(project_id, 8)
        )),
        CommandAction::EnterSession { session_id } => Some(format!(
            "Opening session `{}`.",
            crate::truncate_utf8(session_id, 8)
        )),
        CommandAction::AttachToSession { session_id } => Some(format!(
            "Attached this chat to session `{}`.",
            crate::truncate_utf8(session_id, 8)
        )),
        CommandAction::DetachFromSession => Some("Detached this chat from its session.".into()),
        CommandAction::HandoverToChannel {
            channel_id,
            account_id,
            chat_id,
            thread_id,
            ..
        } => {
            let thread = thread_id
                .as_deref()
                .map(|t| format!(":{}", t))
                .unwrap_or_default();
            Some(format!(
                "Handed this session over to `{channel_id}:{account_id}:{chat_id}{thread}`."
            ))
        }
        _ => None,
    }
}

const HISTORY_PICKER_LIMIT: usize = 20;

fn render_model_picker_history(
    models: &[ModelPickerItem],
    active_provider_id: Option<&str>,
    active_model_id: Option<&str>,
) -> String {
    let mut lines = vec![format!("**Models** ({})", models.len())];
    for item in models.iter().take(HISTORY_PICKER_LIMIT) {
        let active = active_provider_id == Some(item.provider_id.as_str())
            && active_model_id == Some(item.model_id.as_str());
        let marker = if active { " (active)" } else { "" };
        lines.push(format!(
            "- **{}** / {} `{}`{}",
            item.provider_name, item.model_name, item.model_id, marker
        ));
    }
    append_truncated_note(&mut lines, models.len());
    lines.join("\n")
}

fn render_project_picker_history(projects: &[ProjectPickerItem]) -> String {
    let mut lines = vec![format!("**Projects** ({})", projects.len())];
    for item in projects.iter().take(HISTORY_PICKER_LIMIT) {
        let desc = item
            .description
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!(" - {}", crate::truncate_utf8(s, 120)))
            .unwrap_or_default();
        lines.push(format!(
            "- **{}** - {} session(s){}",
            item.name, item.session_count, desc
        ));
    }
    append_truncated_note(&mut lines, projects.len());
    lines.join("\n")
}

fn render_session_picker_history(sessions: &[SessionPickerItem]) -> String {
    let mut lines = vec![format!("**Sessions** ({})", sessions.len())];
    for item in sessions.iter().take(HISTORY_PICKER_LIMIT) {
        let id_short = crate::truncate_utf8(&item.id, 8);
        let mut chips = Vec::new();
        if !item.agent_label.is_empty() {
            chips.push(format!("agent: {}", item.agent_label));
        }
        if let Some(project) = item.project_label.as_deref().filter(|s| !s.is_empty()) {
            chips.push(format!("project: {}", project));
        }
        if let Some(channel) = item.channel_label.as_deref().filter(|s| !s.is_empty()) {
            chips.push(channel.to_string());
        }
        let suffix = if chips.is_empty() {
            String::new()
        } else {
            format!(" - {}", chips.join(" / "))
        };
        lines.push(format!("- `{}` **{}**{}", id_short, item.title, suffix));
        if let Some(snippet) = item.snippet.as_deref().filter(|s| !s.is_empty()) {
            lines.push(format!("  > {}", snippet));
        }
    }
    append_truncated_note(&mut lines, sessions.len());
    lines.join("\n")
}

fn append_truncated_note(lines: &mut Vec<String>, total: usize) {
    if total > HISTORY_PICKER_LIMIT {
        lines.push(format!("- ... and {} more", total - HISTORY_PICKER_LIMIT));
    }
}

/// PassThrough slash commands become real user turns, so they must keep the
/// normal message/context path. `/new` creates a different conversation and
/// should not seed the old or fresh transcript with a control event.
pub fn should_persist_slash_history(action: Option<&CommandAction>) -> bool {
    !matches!(
        action,
        Some(CommandAction::PassThrough { .. })
            | Some(CommandAction::NewSession { .. })
            | Some(CommandAction::SwitchAgent { .. })
    )
}

/// Quick check if text is a slash command.
pub fn is_slash_command(text: String) -> bool {
    parser::is_command(&text)
}

/// Hard upper bound the IM bot menus enforce on themselves: Telegram caps
/// `setMyCommands` at 100 entries, Discord caps global application commands
/// at 100. Truncated tail is still callable by users typing manually — just
/// hidden from the platform's menu/auto-complete UI.
pub const IM_MENU_HARD_CAP: usize = 100;

/// Snapshot of the slash commands an IM channel should publish to its bot
/// menu — `registry::all_commands()` plus invocable skills (collision-resolved),
/// minus `IM_DISABLED_COMMANDS`, capped at `IM_MENU_HARD_CAP`.
///
/// Single source-of-truth for both Telegram (`setMyCommands`) and Discord
/// (`bulk_overwrite_global_commands`); the platform-specific layers project
/// each `SlashCommandDef` into their own wire format. `description_en()`
/// gives a stable English label both platforms can render.
pub async fn im_menu_entries() -> Vec<SlashCommandDef> {
    let defs = match list_slash_commands().await {
        Ok(v) => v,
        Err(e) => {
            crate::app_warn!(
                "channel",
                "menu_sync",
                "list_slash_commands failed: {} — falling back to built-in only",
                e
            );
            registry::all_commands()
        }
    };

    let mut entries: Vec<SlashCommandDef> = defs
        .into_iter()
        .filter(|cmd| !registry::is_im_disabled(&cmd.name))
        .collect();

    if entries.len() > IM_MENU_HARD_CAP {
        crate::app_warn!(
            "channel",
            "menu_sync",
            "Slash command count {} exceeds IM menu cap {} — truncating tail",
            entries.len(),
            IM_MENU_HARD_CAP
        );
        entries.truncate(IM_MENU_HARD_CAP);
    }

    entries
}

/// Truncate a description to `max_chars` characters, appending "…" if truncated.
pub(crate) fn truncate_description(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars - 1).collect();
    format!("{}…", truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{SkillDisplay, SkillEntry, SkillRequires, SkillStatus};

    fn load_session_title(
        db: &crate::session::SessionDB,
        session_id: &str,
    ) -> (Option<String>, String) {
        let conn = db.conn.lock().expect("lock db");
        conn.query_row(
            "SELECT title, title_source FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("load session title")
    }

    fn mk_skill(name: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            aliases: Vec::new(),
            description: "stub".into(),
            when_to_use: None,
            source: "managed".into(),
            file_path: format!("/tmp/{name}/SKILL.md"),
            base_dir: format!("/tmp/{name}"),
            requires: SkillRequires::default(),
            skill_key: None,
            user_invocable: None,
            disable_model_invocation: None,
            command_dispatch: None,
            command_tool: None,
            command_arg_mode: None,
            command_arg_placeholder: None,
            command_arg_options: None,
            command_prompt_template: None,
            install: Vec::new(),
            allowed_tools: Vec::new(),
            context_mode: None,
            agent: None,
            effort: None,
            paths: None,
            status: SkillStatus::Active,
            authored_by: None,
            rationale: None,
            display: SkillDisplay::default(),
        }
    }

    #[test]
    fn silent_aliases_are_reserved_against_skills() {
        // `/reasoning` and `/think` are silent dispatch aliases — not in
        // the registry, but must still block same-named skills from resolving
        // to those names (otherwise the skill is silently shadowed by the
        // built-in dispatch).
        let reserved = builtin_command_names();
        assert!(reserved.contains("reason"));
        assert!(reserved.contains("reasoning"));
        assert!(reserved.contains("thinking"));
        assert!(reserved.contains("think"));
        assert_eq!(canonical_builtin_command_name("think"), "thinking");
        assert_eq!(canonical_builtin_command_name("reasoning"), "reason");

        let skill = mk_skill("reasoning");
        let resolved = resolve_skill_command_names(std::slice::from_ref(&skill), reserved);
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].typed_name, "reasoning_skill",
            "skill must not collide with silent built-in alias"
        );

        let skill = mk_skill("think");
        let resolved = resolve_skill_command_names(std::slice::from_ref(&skill), reserved);
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].typed_name, "think_skill",
            "skill must not collide with silent built-in alias"
        );
    }

    #[test]
    fn slash_history_events_are_event_rows_with_user_display_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.db");
        let db = crate::session::SessionDB::open(&path).expect("open");
        let meta = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");

        let ids = append_slash_history_events(
            &db,
            &meta.id,
            "/status",
            Some("All systems nominal."),
            crate::chat_engine::ChatSource::Desktop,
        )
        .expect("append slash history");
        assert_eq!(ids.len(), 2);

        let messages = db.load_session_messages(&meta.id).expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, crate::session::MessageRole::Event);
        assert_eq!(messages[0].content, "/status");
        assert!(messages[0]
            .attachments_meta
            .as_deref()
            .expect("command meta")
            .contains("\"displayAs\":\"user\""));
        assert_eq!(messages[1].role, crate::session::MessageRole::Event);
        assert_eq!(messages[1].content, "All systems nominal.");
        assert!(messages[1]
            .attachments_meta
            .as_deref()
            .expect("result meta")
            .contains("\"kind\":\"result\""));
        let (title, title_source) = load_session_title(&db, &meta.id);
        assert_eq!(title.as_deref(), Some("/status"));
        assert_eq!(
            title_source,
            crate::session_title::TITLE_SOURCE_FIRST_MESSAGE
        );
    }

    #[test]
    fn slash_history_result_events_persist_structured_action_fallback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.db");
        let db = crate::session::SessionDB::open(&path).expect("open");
        let meta = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let result = CommandResult {
            content: String::new(),
            action: Some(CommandAction::ShowModelPicker {
                models: vec![ModelPickerItem {
                    provider_id: "p1".into(),
                    provider_name: "OpenAI".into(),
                    model_id: "gpt-test".into(),
                    model_name: "GPT Test".into(),
                    input_types: vec!["text".into(), "image".into()],
                }],
                active_provider_id: Some("p1".into()),
                active_model_id: Some("gpt-test".into()),
            }),
        };

        let ids = append_slash_history_result_events(
            &db,
            &meta.id,
            "/model",
            &result,
            crate::chat_engine::ChatSource::Desktop,
        )
        .expect("append slash history");
        assert_eq!(ids.len(), 2);

        let messages = db.load_session_messages(&meta.id).expect("messages");
        assert_eq!(messages[0].content, "/model");
        assert!(messages[1].content.contains("**Models** (1)"));
        assert!(messages[1].content.contains("GPT Test"));
        assert!(messages[1].content.contains("(active)"));
    }

    #[test]
    fn slash_history_loop_create_hides_slash_prefix_and_result() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.db");
        let db = crate::session::SessionDB::open(&path).expect("open");
        let meta = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let result = CommandResult {
            content: "Loop created.\n\nImmediate first run: queued.".to_string(),
            action: Some(CommandAction::DisplayOnly),
        };

        let ids = append_slash_history_result_events(
            &db,
            &meta.id,
            "/loop every 10m: check release notes",
            &result,
            crate::chat_engine::ChatSource::Desktop,
        )
        .expect("append slash history");
        assert_eq!(ids.len(), 1);

        let messages = db.load_session_messages(&meta.id).expect("messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "every 10m: check release notes");
        let command_meta = messages[0]
            .attachments_meta
            .as_deref()
            .expect("command meta");
        assert!(command_meta.contains("\"displayAs\":\"user\""));
        assert!(command_meta.contains("\"mode\":\"loop\""));
        let (title, _) = load_session_title(&db, &meta.id);
        assert_eq!(title.as_deref(), Some("every 10m: check release notes"));
    }

    #[test]
    fn command_action_serializes_variant_fields_as_camel_case() {
        let recap = serde_json::to_value(CommandAction::RecapCard {
            report_id: "report-1".into(),
        })
        .expect("serialize recap action");
        assert_eq!(
            recap,
            serde_json::json!({
                "type": "recapCard",
                "reportId": "report-1",
            })
        );

        let skill = serde_json::to_value(CommandAction::SkillFork {
            run_id: "run-1".into(),
            skill_name: "drawio".into(),
        })
        .expect("serialize skill action");
        assert_eq!(
            skill,
            serde_json::json!({
                "type": "skillFork",
                "runId": "run-1",
                "skillName": "drawio",
            })
        );
    }

    #[test]
    fn pass_through_slash_history_stays_on_normal_chat_path() {
        assert!(!should_persist_slash_history(Some(
            &CommandAction::PassThrough {
                message: "expanded".into(),
            },
        )));
        assert!(should_persist_slash_history(Some(
            &CommandAction::DisplayOnly
        )));
        assert!(should_persist_slash_history(None));
    }

    #[test]
    fn goal_slash_history_does_not_strip_objective_prefix_words() {
        assert_eq!(
            goal_command_display_text("pause react upgrade"),
            Some("pause react upgrade".to_string())
        );
        assert_eq!(
            goal_command_display_text("update react upgrade"),
            Some("update react upgrade".to_string())
        );
        assert_eq!(
            goal_command_display_text("set react upgrade"),
            Some("set react upgrade".to_string())
        );
        assert_eq!(
            goal_command_display_text("pause"),
            Some("Pause active goal".to_string())
        );
    }
}
