pub mod fuzzy;
pub mod handlers;
pub mod parser;
pub mod registry;
pub mod types;

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::skills::SkillEntry;
use types::{CommandCategory, CommandResult, SlashCommandDef};

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
const SILENT_BUILTIN_ALIASES: &[&str] = &["reasoning"];

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

    app_info!(
        "slash_cmd",
        "dispatch",
        "Executing /{} args={:?}",
        name,
        args
    );

    let result = handlers::dispatch(session_id.as_deref(), &agent_id, &name, &args).await?;

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
        // `/reasoning` is a silent dispatch alias for `/reason` — not in
        // the registry, but must still block a same-named skill from
        // resolving as `/reasoning` (otherwise the skill is silently
        // shadowed by the built-in dispatch).
        let reserved = builtin_command_names();
        assert!(reserved.contains("reason"));
        assert!(reserved.contains("reasoning"));

        let skill = mk_skill("reasoning");
        let resolved = resolve_skill_command_names(std::slice::from_ref(&skill), reserved);
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].typed_name, "reasoning_skill",
            "skill must not collide with silent built-in alias"
        );
    }
}
