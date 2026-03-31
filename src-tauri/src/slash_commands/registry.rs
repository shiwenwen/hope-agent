use super::types::{CommandCategory, SlashCommandDef};

/// Returns all available slash command definitions.
pub fn all_commands() -> Vec<SlashCommandDef> {
    vec![
        // ── Session ──
        SlashCommandDef {
            name: "new".into(),
            category: CommandCategory::Session,
            description_key: "slashCommands.new.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "clear".into(),
            category: CommandCategory::Session,
            description_key: "slashCommands.clear.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "compact".into(),
            category: CommandCategory::Session,
            description_key: "slashCommands.compact.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "stop".into(),
            category: CommandCategory::Session,
            description_key: "slashCommands.stop.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "rename".into(),
            category: CommandCategory::Session,
            description_key: "slashCommands.rename.description".into(),
            has_args: true,
            arg_placeholder: Some("<title>".into()),
            arg_options: None,
            description_raw: None,
        },
        // ── Model ──
        SlashCommandDef {
            name: "model".into(),
            category: CommandCategory::Model,
            description_key: "slashCommands.model.description".into(),
            has_args: true,
            arg_placeholder: Some("[name]".into()),
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "think".into(),
            category: CommandCategory::Model,
            description_key: "slashCommands.think.description".into(),
            has_args: true,
            arg_placeholder: Some("<level>".into()),
            arg_options: Some(vec![
                "off".into(),
                "low".into(),
                "medium".into(),
                "high".into(),
            ]),
            description_raw: None,
        },
        // ── Memory ──
        SlashCommandDef {
            name: "remember".into(),
            category: CommandCategory::Memory,
            description_key: "slashCommands.remember.description".into(),
            has_args: true,
            arg_placeholder: Some("<text>".into()),
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "forget".into(),
            category: CommandCategory::Memory,
            description_key: "slashCommands.forget.description".into(),
            has_args: true,
            arg_placeholder: Some("<query>".into()),
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "memories".into(),
            category: CommandCategory::Memory,
            description_key: "slashCommands.memories.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        // ── Agent ──
        SlashCommandDef {
            name: "agent".into(),
            category: CommandCategory::Agent,
            description_key: "slashCommands.agent.description".into(),
            has_args: true,
            arg_placeholder: Some("<name>".into()),
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "agents".into(),
            category: CommandCategory::Agent,
            description_key: "slashCommands.agents.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        // ── Plan ──
        SlashCommandDef {
            name: "plan".into(),
            category: CommandCategory::Session,
            description_key: "slashCommands.plan.description".into(),
            has_args: true,
            arg_placeholder: Some("[exit|show|approve]".into()),
            arg_options: Some(vec!["exit".into(), "show".into(), "approve".into()]),
            description_raw: None,
        },
        // ── Utility ──
        SlashCommandDef {
            name: "permission".into(),
            category: CommandCategory::Utility,
            description_key: "slashCommands.permission.description".into(),
            has_args: true,
            arg_placeholder: Some("<mode>".into()),
            arg_options: Some(vec!["auto".into(), "ask".into(), "full".into()]),
            description_raw: None,
        },
        SlashCommandDef {
            name: "help".into(),
            category: CommandCategory::Utility,
            description_key: "slashCommands.help.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "status".into(),
            category: CommandCategory::Utility,
            description_key: "slashCommands.status.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "export".into(),
            category: CommandCategory::Utility,
            description_key: "slashCommands.export.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "usage".into(),
            category: CommandCategory::Utility,
            description_key: "slashCommands.usage.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "search".into(),
            category: CommandCategory::Utility,
            description_key: "slashCommands.search.description".into(),
            has_args: true,
            arg_placeholder: Some("<query>".into()),
            arg_options: None,
            description_raw: None,
        },
        SlashCommandDef {
            name: "prompts".into(),
            category: CommandCategory::Utility,
            description_key: "slashCommands.prompts.description".into(),
            has_args: false,
            arg_placeholder: None,
            arg_options: None,
            description_raw: None,
        },
    ]
}

/// Check if a command name is valid.
#[allow(dead_code)]
pub fn is_valid_command(name: &str) -> bool {
    all_commands().iter().any(|c| c.name == name)
}
