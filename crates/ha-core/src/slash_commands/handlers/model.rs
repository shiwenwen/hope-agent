use crate::config::AppConfig;
use crate::provider::{self, AvailableModel};
use crate::slash_commands::fuzzy;
use crate::slash_commands::types::{CommandAction, CommandResult, ModelPickerItem};

/// /model [name] — List or switch models.
pub fn handle_model(store: &AppConfig, args: &str) -> Result<CommandResult, String> {
    let models = provider::build_available_models(&store.providers);

    if args.trim().is_empty() {
        // List all available models as an interactive picker
        if models.is_empty() {
            return Ok(CommandResult {
                content: "No models available. Please configure a provider first.".into(),
                action: Some(CommandAction::DisplayOnly),
            });
        }

        let items: Vec<ModelPickerItem> = models
            .iter()
            .map(|m| ModelPickerItem {
                provider_id: m.provider_id.clone(),
                provider_name: m.provider_name.clone(),
                model_id: m.model_id.clone(),
                model_name: m.model_name.clone(),
            })
            .collect();

        let (active_pid, active_mid) = store
            .active_model
            .as_ref()
            .map(|a| (Some(a.provider_id.clone()), Some(a.model_id.clone())))
            .unwrap_or((None, None));

        return Ok(CommandResult {
            content: String::new(),
            action: Some(CommandAction::ShowModelPicker {
                models: items,
                active_provider_id: active_pid,
                active_model_id: active_mid,
            }),
        });
    }

    let matched = fuzzy::fuzzy_match_one(
        &models,
        args,
        |m: &AvailableModel| vec![m.model_name.clone(), m.model_id.clone()],
        |m: &AvailableModel| m.model_name.clone(),
        "model",
    )?;

    Ok(CommandResult {
        content: format!(
            "Switched to **{}** / {}",
            matched.provider_name, matched.model_name
        ),
        action: Some(CommandAction::SwitchModel {
            provider_id: matched.provider_id.clone(),
            model_id: matched.model_id.clone(),
        }),
    })
}

/// /think <level> — Set reasoning effort.
pub fn handle_think(args: &str) -> Result<CommandResult, String> {
    let level = args.trim().to_lowercase();
    let valid = ["off", "none", "low", "medium", "high", "xhigh"];
    let effort = if level == "off" || level == "none" {
        "none".to_string()
    } else if valid.contains(&level.as_str()) {
        level
    } else {
        return Err(format!(
            "Invalid thinking level: `{}`. Use: off, low, medium, high, xhigh",
            args.trim()
        ));
    };

    Ok(CommandResult {
        content: format!("Thinking effort set to **{}**", effort),
        action: Some(CommandAction::SetEffort { effort }),
    })
}
