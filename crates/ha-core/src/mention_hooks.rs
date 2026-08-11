//! Extensible resolution for typed Plugin/Connector mentions.
//!
//! Providers may return only bounded capability metadata. Authority and prompt
//! placement remain sealed in `prompt_context`, and selecting a capability
//! never calls a tool, authorizes disclosure, or installs/logs in anything.

use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::prompt_context::MentionKind;

const MAX_PROVIDER_CANDIDATES: usize = 100;
const MAX_ALIAS_BYTES: usize = 256;
const MAX_SUMMARY_BYTES: usize = 600;

#[derive(Debug, Clone)]
pub struct ResolvedCapabilityMention {
    pub namespace: String,
    pub display_alias: String,
    pub capability_summary: String,
}

/// Bounded, non-sensitive picker row exposed by a registered capability
/// provider. This is discovery metadata only: it carries no credentials,
/// authorization decision, tool schema, remote instructions, or execution
/// capability.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MentionCapabilityCandidate {
    pub kind: MentionKind,
    pub target_id: String,
    pub display_label: String,
    pub namespace: String,
    pub summary: String,
}

#[derive(Clone, Copy)]
pub struct MentionProvider {
    pub namespace: &'static str,
    pub list: fn(principal_agent_id: &str) -> Vec<MentionCapabilityCandidate>,
    pub resolve: fn(
        kind: MentionKind,
        target_id: &str,
        principal_agent_id: &str,
    ) -> Option<ResolvedCapabilityMention>,
}

static PROVIDERS: LazyLock<RwLock<Vec<MentionProvider>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

pub fn register_mention_provider(provider: MentionProvider) -> anyhow::Result<()> {
    if provider.namespace.trim().is_empty() {
        anyhow::bail!("mention provider namespace cannot be empty");
    }
    let mut providers = PROVIDERS
        .write()
        .map_err(|_| anyhow::anyhow!("mention provider registry is poisoned"))?;
    if providers
        .iter()
        .any(|existing| existing.namespace == provider.namespace)
    {
        anyhow::bail!(
            "mention provider namespace '{}' is already registered",
            provider.namespace
        );
    }
    providers.push(provider);
    providers.sort_by_key(|provider| provider.namespace);
    Ok(())
}

pub(crate) fn resolve_capability_mention(
    kind: MentionKind,
    target_id: &str,
    principal_agent_id: &str,
) -> Option<ResolvedCapabilityMention> {
    let (namespace, provider_target_id) = target_id.split_once("::")?;
    let providers = PROVIDERS.read().ok()?;
    providers
        .iter()
        .find(|provider| provider.namespace == namespace)
        .and_then(|provider| (provider.resolve)(kind, provider_target_id, principal_agent_id))
}

/// List typed Plugin/Connector picker candidates from every registered
/// provider. The registry owns ordering, deduplication, caps, and string
/// bounds so a feature provider cannot flood the composer or leak arbitrary
/// remote metadata through this trusted local discovery surface.
pub fn list_capability_mentions(principal_agent_id: &str) -> Vec<MentionCapabilityCandidate> {
    let Ok(providers) = PROVIDERS.read() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for provider in providers.iter() {
        for mut row in (provider.list)(principal_agent_id)
            .into_iter()
            .take(MAX_PROVIDER_CANDIDATES)
        {
            let provider_target_id = row.target_id.trim();
            if !matches!(row.kind, MentionKind::Plugin | MentionKind::Connector)
                || provider_target_id.is_empty()
            {
                continue;
            }
            row.namespace = provider.namespace.to_string();
            row.target_id = format!("{}::{provider_target_id}", provider.namespace);
            if !seen.insert((row.kind, row.target_id.clone())) {
                continue;
            }
            row.display_label =
                crate::truncate_utf8(&row.display_label, MAX_ALIAS_BYTES).to_string();
            row.summary = crate::truncate_utf8(&row.summary, MAX_SUMMARY_BYTES).to_string();
            rows.push(row);
        }
    }
    rows.sort_by(|left, right| {
        format!("{:?}:{}", left.kind, left.display_label.to_lowercase()).cmp(&format!(
            "{:?}:{}",
            right.kind,
            right.display_label.to_lowercase()
        ))
    });
    rows
}
