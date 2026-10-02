use crate::config::AppConfig;
use crate::provider::{self, ActiveModel, AvailableModel};
use crate::slash_commands::fuzzy;
use crate::slash_commands::types::{CommandAction, CommandResult, ModelPickerItem};

/// /model [name] — List or switch models.
///
/// `effective_model` is the conversation's resolved model — session pin →
/// Agent primary → global `active_model`, computed by the dispatcher with
/// the same configured-chain order the next turn uses (#786): the picker's
/// ✓ must show what the session actually runs with, not whichever tier the
/// caller happened to read.
pub fn handle_model(
    store: &AppConfig,
    args: &str,
    effective_model: Option<&ActiveModel>,
) -> Result<CommandResult, String> {
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
                input_types: m.input_types.clone(),
            })
            .collect();

        // The checkmark reads the resolved effective model: switching via the
        // picker pins to the session, so it must match the source the next
        // turn's dispatch will resolve.
        let (active_pid, active_mid) = effective_model
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

/// /thinking <level> — Set reasoning effort.
pub fn handle_think(args: &str) -> Result<CommandResult, String> {
    let level = args.trim().to_lowercase();
    let valid = [
        "off", "none", "minimal", "low", "medium", "high", "xhigh", "max",
    ];
    let effort = if level == "off" || level == "none" {
        "none".to_string()
    } else if valid.contains(&level.as_str()) {
        level
    } else {
        return Err(format!(
            "Invalid thinking level: `{}`. Use: off, minimal, low, medium, high, xhigh, max",
            args.trim()
        ));
    };

    Ok(CommandResult {
        content: format!("Thinking effort set to **{}**", effort),
        action: Some(CommandAction::SetEffort { effort }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ApiType, ModelConfig, ProviderConfig};

    fn store_with_models(active: Option<(&str, &str)>) -> AppConfig {
        let mut provider = ProviderConfig::new(
            "Provider One".into(),
            ApiType::OpenaiChat,
            "https://example.test".into(),
            "test-key".into(),
        );
        provider.id = "p1".into();
        provider.enabled = true;
        provider.models = vec![
            ModelConfig {
                id: "m1".into(),
                name: "Model One".into(),
                input_types: vec!["text".into()],
                context_window: 128_000,
                max_tokens: 8192,
                reasoning: false,
                thinking_style: None,
                cost_input: None,
                cost_output: None,
            },
            ModelConfig {
                id: "m2".into(),
                name: "Model Two".into(),
                input_types: vec!["text".into()],
                context_window: 64_000,
                max_tokens: 8192,
                reasoning: false,
                thinking_style: None,
                cost_input: None,
                cost_output: None,
            },
        ];
        AppConfig {
            providers: vec![provider],
            active_model: active.map(|(provider_id, model_id)| ActiveModel {
                provider_id: provider_id.into(),
                model_id: model_id.into(),
            }),
            ..Default::default()
        }
    }

    fn expect_picker(result: CommandResult) -> (Option<String>, Option<String>) {
        match result.action.expect("picker action") {
            CommandAction::ShowModelPicker {
                active_provider_id,
                active_model_id,
                ..
            } => (active_provider_id, active_model_id),
            other => panic!("expected ShowModelPicker, got {other:?}"),
        }
    }

    #[test]
    fn picker_checkmark_shows_the_resolved_effective_model() {
        let store = store_with_models(Some(("p1", "m1")));
        let session_pin = ActiveModel {
            provider_id: "p1".into(),
            model_id: "m2".into(),
        };

        let result = handle_model(&store, "", Some(&session_pin)).expect("ok");
        let (active_pid, active_mid) = expect_picker(result);
        assert_eq!(active_pid.as_deref(), Some("p1"));
        assert_eq!(active_mid.as_deref(), Some("m2"));
    }

    // No-pin fallback (session pin → Agent primary → global) is resolved by
    // `effective_session_model` in the dispatcher and covered by its tests in
    // handlers::mod; `handle_model` only displays what it receives.

    #[test]
    fn picker_checkmark_clears_without_an_effective_model() {
        let store = store_with_models(None);

        let result = handle_model(&store, "", None).expect("ok");
        let (active_pid, active_mid) = expect_picker(result);
        assert_eq!(active_pid, None);
        assert_eq!(active_mid, None);
    }

    #[test]
    fn picker_checkmark_still_clears_with_a_session_pin_but_no_global() {
        let store = store_with_models(None);
        let session_pin = ActiveModel {
            provider_id: "p1".into(),
            model_id: "m2".into(),
        };

        let result = handle_model(&store, "", Some(&session_pin)).expect("ok");
        let (active_pid, active_mid) = expect_picker(result);
        assert_eq!(active_pid.as_deref(), Some("p1"));
        assert_eq!(active_mid.as_deref(), Some("m2"));
    }
}
