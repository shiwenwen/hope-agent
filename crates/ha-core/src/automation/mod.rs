//! Shared execution path for background/automation LLM tasks — the "model"
//! side of the model-vs-Agent split (see `docs/architecture/automation-model.md`).
//!
//! Recap, Dreaming, Knowledge Compile, Skills auto_review, the Hooks `prompt`
//! handler, Smart mode judge, session title, memory extraction, and the
//! compaction summarizer all resolve their model chain the same way and run
//! through the same [`run`] entry point, instead of each hand-rolling
//! "construct an `AssistantAgent` + call `side_query`" and silently getting
//! no retry (see the doc comment on [`run`] for why that mattered).

use anyhow::{anyhow, Result};

use crate::agent::AssistantAgent;
use crate::config::AppConfig;
use crate::provider::{find_provider, ActiveModel, ModelChain};

/// Resolve the effective model chain for a background task.
///
/// Priority: `override_chain` (the caller's own config — a new
/// `model_override` field, or a legacy field parsed into an equivalent chain
/// on the fly) → `config.function_models.automation` (the shared automation
/// default) → `config.active_model` / `config.fallback_models` (the chat
/// model chain, so a fresh install with zero extra config still works).
/// Returns an empty `Vec` if nothing resolves — callers should surface a
/// clear "no model configured" error rather than silently doing nothing.
///
/// Each tier is filtered to candidates whose provider still exists/is
/// enabled before being accepted — a tier that resolves to only dead
/// providers (most commonly a deprecated single-colon legacy string, which
/// carries no existence check of its own by design; see
/// [`parse_legacy_model_string`]) falls through to the next tier instead of
/// being returned as a chain `run`/`run_vision` can only fail on. This
/// restores the graceful-degradation behavior the per-consumer legacy
/// fallback-agent helpers (deleted when consumers migrated to this module)
/// used to provide individually.
pub fn effective_chain(config: &AppConfig, override_chain: Option<ModelChain>) -> Vec<ActiveModel> {
    if let Some(chain) = override_chain {
        let live = filter_live_candidates(config, chain.into_vec());
        if !live.is_empty() {
            return live;
        }
    }
    if let Some(chain) = config.function_models.automation.clone() {
        let live = filter_live_candidates(config, chain.into_vec());
        if !live.is_empty() {
            return live;
        }
    }
    let mut chain = Vec::new();
    if let Some(active) = config.active_model.clone() {
        chain.push(active);
    }
    chain.extend(config.fallback_models.iter().cloned());
    chain
}

/// Drops candidates whose provider no longer exists/is disabled. See
/// [`effective_chain`] for why this matters at the tier-selection level, not
/// just inside `run`/`run_vision`'s own per-candidate execution loop.
fn filter_live_candidates(config: &AppConfig, chain: Vec<ActiveModel>) -> Vec<ActiveModel> {
    chain
        .into_iter()
        .filter(|c| find_provider(&config.providers, &c.provider_id).is_some())
        .collect()
}

/// Resolve a deprecated `agent_id`-style config field to an equivalent
/// [`ModelChain`] by reading that agent's own model config — the same
/// resolution `crate::provider::resolve_model_chain` already does for
/// regular chat, just materialized once instead of keeping an Agent
/// indirection alive. Shared by every Phase 1 consumer that used to borrow
/// an agent id (Recap, Knowledge Compile) so the "load agent → resolve its
/// model chain" logic isn't duplicated per consumer.
///
/// Returns `None` if `agent_id` is empty/whitespace or can't be loaded —
/// callers should fall through to the automation default rather than
/// hard-failing (a deleted/renamed agent shouldn't break the feature that
/// used to borrow its model).
pub fn resolve_legacy_agent_chain(config: &AppConfig, agent_id: &str) -> Option<ModelChain> {
    let agent_id = agent_id.trim();
    if agent_id.is_empty() {
        return None;
    }
    let agent_def = crate::agent_loader::load_agent(agent_id).ok()?;
    let (primary, fallbacks) =
        crate::provider::resolve_model_chain(&agent_def.config.model, config);
    Some(ModelChain {
        primary: primary?,
        fallbacks,
    })
}

/// Parse a deprecated single-colon `"provider_id:model_id"` string (the
/// shape Dreaming / Skills auto_review / Hooks `prompt` handlers / the
/// compaction summarizer all used) into a single-entry [`ModelChain`] (no
/// fallbacks — these fields never carried fallback semantics). Returns
/// `None` if empty/whitespace or malformed, so callers fall through to the
/// automation default instead of hard-failing on a typo'd legacy value.
///
/// Deliberately **not** [`crate::provider::parse_model_ref`], which expects
/// the double-colon `"provider_id::model_id"` separator used by
/// `AgentModelConfig.primary`/`fallbacks` — these legacy fields all used a
/// single colon, an inconsistency predating this module that isn't worth
/// silently "fixing" (it would just as silently break anyone's existing
/// single-colon value).
pub fn parse_legacy_model_string(value: &str) -> Option<ModelChain> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let (provider_id, model_id) = value.split_once(':')?;
    if provider_id.is_empty() || model_id.is_empty() {
        return None;
    }
    Some(ModelChain {
        primary: ActiveModel {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        },
        fallbacks: Vec::new(),
    })
}

/// Best-effort "Provider Name / Model Name" display label; falls back to the
/// raw `provider_id::model_id` form when the provider/model can't be looked
/// up (e.g. deleted after the label's original resolution).
pub fn model_label(config: &AppConfig, model: &ActiveModel) -> String {
    let Some(provider) = find_provider(&config.providers, &model.provider_id) else {
        return model.to_string();
    };
    let model_name = provider
        .models
        .iter()
        .find(|m| m.id == model.model_id)
        .map(|m| m.name.as_str())
        .unwrap_or(&model.model_id);
    format!("{} / {}", provider.name, model_name)
}

/// Spec for a one-shot background model call. See [`run`].
pub struct ModelTaskSpec<'a> {
    /// Stable tag for the `model_usage_events.operation` column (e.g.
    /// `"recap.facets"`, `"dreaming.narrative"`) — lets the Dashboard break
    /// down background-task cost by consumer instead of one undifferentiated
    /// `agent.side_query` bucket.
    pub purpose: &'static str,
    /// Resolved candidates, in try-order. Build with [`effective_chain`].
    pub chain: Vec<ActiveModel>,
    /// Key for `PROFILE_STICKY` / `PROFILE_COOLDOWNS` bookkeeping and the
    /// usage ledger's `session_id` column. Pass the real session id when one
    /// exists; otherwise a stable synthetic key (e.g.
    /// `"automation:recap.facets"`) so this task's profile rotation still
    /// gets cooldown/sticky affinity instead of silently skipping failover
    /// (an unset `session_id` is exactly what caused today's zero-retry bug).
    pub session_key: &'a str,
    pub instruction: &'a str,
    pub max_tokens: u32,
}

/// Result of a successful [`run`]/[`run_vision`] call.
pub struct ModelTaskOutput {
    pub text: String,
    /// The candidate that actually produced `text` — not necessarily
    /// `chain[0]`; may be a fallback. Callers that persist a "generated by"
    /// label should read this instead of pre-computing one from `chain[0]`
    /// before the call, which silently mis-attributes the label whenever a
    /// fallback fires.
    pub model: ActiveModel,
}

/// Shared per-candidate setup: resolve provider, construct agent, wire
/// failover context + session id. Used by both [`run`] and [`run_vision`] —
/// this setup is identical between them; only the final query call (text vs.
/// vision-with-attachments) differs. Extracting it here is what prevents the
/// "someone adds a new one-shot path and forgets `set_session_id`" failure
/// mode — exactly the bug class this whole module exists to close.
async fn build_candidate_agent(
    config: &AppConfig,
    candidate: &ActiveModel,
    session_key: &str,
) -> Result<AssistantAgent> {
    let provider = find_provider(&config.providers, &candidate.provider_id).ok_or_else(|| {
        anyhow!(
            "provider '{}' not found or disabled for model '{}'",
            candidate.provider_id,
            candidate.model_id
        )
    })?;
    let mut agent = AssistantAgent::try_new_from_provider(provider, &candidate.model_id).await?;
    agent = agent.with_failover_context(provider);
    agent.set_session_id(session_key);
    Ok(agent)
}

/// Run a one-shot background model task, trying each candidate in
/// `spec.chain` in order until one succeeds.
///
/// This closes the gap that motivated this module: `recap::report`'s old
/// `build_analysis_agent` family (and everything that borrowed it) picked the
/// first *constructible* model once, at agent-construction time, then called
/// `side_query()` on that single agent — which only fails over auth
/// *profiles* of that one model, and only when the agent carries a
/// `session_id`, which the borrowed-agent path never set, so even
/// profile-level retry never fired. A transient error, or the primary model
/// being flat-out misconfigured, failed the whole call. `run` mirrors
/// `chat_engine::engine::run_chat_engine`'s
/// `for model_ref in model_chain { ... continue on failure ... }` loop
/// instead, so a bad/unavailable primary genuinely falls through to the next
/// model in the chain.
pub async fn run(spec: ModelTaskSpec<'_>) -> Result<ModelTaskOutput> {
    if spec.chain.is_empty() {
        return Err(anyhow!(
            "no model configured for '{}' — set a default model in Settings \
             (Model Config's automation default, or the chat default model) \
             before using this feature",
            spec.purpose
        ));
    }

    let config = crate::config::cached_config();
    let mut last_err: Option<anyhow::Error> = None;

    for candidate in &spec.chain {
        let agent = match build_candidate_agent(&config, candidate, spec.session_key).await {
            Ok(agent) => agent,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };

        match agent
            .side_query_with_purpose(spec.purpose, spec.instruction, spec.max_tokens)
            .await
        {
            Ok(result) => {
                return Ok(ModelTaskOutput {
                    text: result.text,
                    model: candidate.clone(),
                })
            }
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        anyhow!(
            "all candidates in the model chain failed for '{}'",
            spec.purpose
        )
    }))
}

/// Spec for a one-shot background VISION-capable model call. See [`run_vision`].
pub struct VisionTaskSpec<'a> {
    pub purpose: &'static str,
    /// Resolved candidates, in try-order. May freely mix vision-capable and
    /// text-only models (e.g. a chain shared with other automation tasks) —
    /// text-only candidates are skipped, not treated as failures.
    pub chain: Vec<ActiveModel>,
    pub session_key: &'a str,
    /// Scoped system prompt framing the attachments as untrusted source
    /// material, never instructions.
    pub system: &'a str,
    pub instruction: &'a str,
    pub attachments: &'a [crate::agent::Attachment],
    pub max_tokens: u32,
}

/// Vision counterpart to [`run`]: same degradation-loop skeleton, built for
/// attachments + vision-capability filtering from the ground up rather than
/// bolted onto the text path (a shared `ModelTaskSpec` would force every
/// text-only consumer to reason about a field that's always unused for them,
/// and the two paths diverge enough at the final query call — cached-prefix
/// `side_query` vs. attachment-carrying `independent_query_with_attachments`
/// — that combining them risks becoming a confusing two-headed function).
/// Only [`build_candidate_agent`] (provider lookup + agent construction +
/// failover/session wiring) is shared with `run`.
///
/// A candidate in the chain without vision support is skipped, not counted
/// as an attempt — so a chain mixing a preferred vision model with cheaper
/// text-only fallbacks (meant for other automation tasks sharing the same
/// global default chain) degrades sensibly instead of erroring on the first
/// text-only entry it reaches.
pub async fn run_vision(spec: VisionTaskSpec<'_>) -> Result<ModelTaskOutput> {
    if spec.chain.is_empty() {
        return Err(anyhow!(
            "no model configured for '{}' — set a default model in Settings before using this feature",
            spec.purpose
        ));
    }

    let config = crate::config::cached_config();
    let mut last_err: Option<anyhow::Error> = None;
    let mut attempted_vision_capable = false;
    let mut saw_any_live_provider = false;

    for candidate in &spec.chain {
        let Some(provider) = find_provider(&config.providers, &candidate.provider_id) else {
            last_err = Some(anyhow!(
                "provider '{}' not found or disabled for model '{}'",
                candidate.provider_id,
                candidate.model_id
            ));
            continue;
        };
        saw_any_live_provider = true;
        if !provider.model_supports_vision(&candidate.model_id) {
            continue; // not a failure — this candidate just isn't eligible for this task
        }
        attempted_vision_capable = true;

        let agent = match build_candidate_agent(&config, candidate, spec.session_key).await {
            Ok(agent) => agent,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };

        match agent
            .independent_query_with_attachments(
                spec.purpose,
                spec.system,
                spec.instruction,
                spec.attachments,
                spec.max_tokens,
            )
            .await
        {
            Ok(result) => {
                return Ok(ModelTaskOutput {
                    text: result.text,
                    model: candidate.clone(),
                })
            }
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        }
    }

    if !attempted_vision_capable {
        // Only diagnose "no vision-capable model configured" when we
        // actually found a live provider that simply didn't support vision —
        // if every candidate failed at the provider-lookup step instead
        // (e.g. a deleted/disabled provider), that's a different, more
        // actionable problem and misreporting it as a model-choice issue
        // sends the user to fix the wrong thing.
        if saw_any_live_provider {
            return Err(anyhow!(
                "no vision-capable model configured for '{}' — pick a model with image input support",
                spec.purpose
            ));
        }
        return Err(last_err.unwrap_or_else(|| {
            anyhow!(
                "no model configured for '{}' — set a default model in Settings before using this feature",
                spec.purpose
            )
        }));
    }
    Err(last_err.unwrap_or_else(|| {
        anyhow!(
            "all vision-capable candidates in the model chain failed for '{}'",
            spec.purpose
        )
    }))
}
