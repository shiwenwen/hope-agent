use crate::provider::{self, AvailableModel, ProviderStore};
use crate::slash_commands::types::{CommandAction, CommandResult};

/// /model [name] — List or switch models.
pub fn handle_model(store: &ProviderStore, args: &str) -> Result<CommandResult, String> {
    let models = provider::build_available_models(&store.providers);

    if args.trim().is_empty() {
        // List all available models
        if models.is_empty() {
            return Ok(CommandResult {
                content: "No models available. Please configure a provider first.".into(),
                action: Some(CommandAction::DisplayOnly),
            });
        }

        let mut lines = vec!["**Available Models**\n".to_string()];
        let mut current_provider = String::new();
        for m in &models {
            if m.provider_name != current_provider {
                current_provider = m.provider_name.clone();
                lines.push(format!("\n**{}**", current_provider));
            }
            let active = store
                .active_model
                .as_ref()
                .map(|a| a.provider_id == m.provider_id && a.model_id == m.model_id)
                .unwrap_or(false);
            let marker = if active { " ← current" } else { "" };
            lines.push(format!("- `{}`{}", m.model_name, marker));
        }

        return Ok(CommandResult {
            content: lines.join("\n"),
            action: Some(CommandAction::DisplayOnly),
        });
    }

    // Fuzzy match model name
    let query = args.trim().to_lowercase();
    let matched = fuzzy_match_model(&models, &query)?;

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
    let valid = ["off", "none", "low", "medium", "high"];
    let effort = if level == "off" || level == "none" {
        "none".to_string()
    } else if valid.contains(&level.as_str()) {
        level
    } else {
        return Err(format!(
            "Invalid thinking level: `{}`. Use: off, low, medium, high",
            args.trim()
        ));
    };

    Ok(CommandResult {
        content: format!("Thinking effort set to **{}**", effort),
        action: Some(CommandAction::SetEffort { effort }),
    })
}

/// Fuzzy match a model by name, accepting partial matches.
fn fuzzy_match_model(models: &[AvailableModel], query: &str) -> Result<AvailableModel, String> {
    // Try exact match on model_id first
    if let Some(m) = models.iter().find(|m| m.model_id.to_lowercase() == query) {
        return Ok(m.clone());
    }

    // Try exact match on model_name
    if let Some(m) = models
        .iter()
        .find(|m| m.model_name.to_lowercase() == query)
    {
        return Ok(m.clone());
    }

    // Try prefix match on model_name or model_id
    let prefix_matches: Vec<_> = models
        .iter()
        .filter(|m| {
            m.model_name.to_lowercase().starts_with(query)
                || m.model_id.to_lowercase().starts_with(query)
        })
        .collect();

    if prefix_matches.len() == 1 {
        return Ok(prefix_matches[0].clone());
    }

    // Try contains match
    let contains_matches: Vec<_> = models
        .iter()
        .filter(|m| {
            m.model_name.to_lowercase().contains(query)
                || m.model_id.to_lowercase().contains(query)
        })
        .collect();

    if contains_matches.len() == 1 {
        return Ok(contains_matches[0].clone());
    }

    if contains_matches.is_empty() {
        Err(format!("No model matching `{}`", query))
    } else {
        let names: Vec<String> = contains_matches
            .iter()
            .map(|m| format!("`{}`", m.model_name))
            .collect();
        Err(format!(
            "Ambiguous model `{}`. Matches: {}",
            query,
            names.join(", ")
        ))
    }
}
