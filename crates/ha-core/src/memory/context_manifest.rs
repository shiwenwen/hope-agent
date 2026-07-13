//! Content-free observability for Memory UX v2.
//!
//! The manifest deliberately stores only counts, byte/token estimates and
//! short hashes. It must never persist memory text, user queries, evidence
//! quotes or embeddings.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::agent::active_memory::{ActiveMemoryRecall, UsedMemoryRef};

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemoryContentMetric {
    pub present: bool,
    pub bytes: usize,
    pub tokens_estimate: u32,
    pub fingerprint: String,
}

impl MemoryContentMetric {
    pub(crate) fn from_optional(value: Option<&str>) -> Self {
        value
            .map(Self::from_text)
            .unwrap_or_else(|| Self::from_text(""))
    }

    pub(crate) fn from_text(value: &str) -> Self {
        Self {
            present: !value.trim().is_empty(),
            bytes: value.len(),
            tokens_estimate: token_estimate(value.len()),
            fingerprint: fingerprint(value.as_bytes()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StaticMemoryContextManifest {
    pub enabled: bool,
    pub incognito: bool,
    pub legacy_static_memory: bool,
    pub agent_core: MemoryContentMetric,
    pub global_core: MemoryContentMetric,
    pub project_index: MemoryContentMetric,
    pub profile_snapshot_source: MemoryContentMetric,
    pub legacy_static_block: MemoryContentMetric,
    pub legacy_candidate_count: usize,
    pub pinned_claim_candidate_count: usize,
    pub injected_ref_count: usize,
    pub injected_refs_by_origin: BTreeMap<String, usize>,
}

impl StaticMemoryContextManifest {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_sources(
        enabled: bool,
        incognito: bool,
        legacy_static_memory: bool,
        agent_core: Option<&str>,
        global_core: Option<&str>,
        project_index: Option<&str>,
        profile_snapshot: Option<&str>,
        legacy_static_block: Option<&str>,
        legacy_candidate_count: usize,
        pinned_claim_candidate_count: usize,
        refs: &[UsedMemoryRef],
    ) -> Self {
        let mut injected_refs_by_origin = BTreeMap::new();
        for reference in refs.iter().filter(|reference| reference.role == "injected") {
            *injected_refs_by_origin
                .entry(reference.origin.clone())
                .or_insert(0) += 1;
        }
        Self {
            enabled,
            incognito,
            legacy_static_memory,
            agent_core: MemoryContentMetric::from_optional(agent_core),
            global_core: MemoryContentMetric::from_optional(global_core),
            project_index: MemoryContentMetric::from_optional(project_index),
            profile_snapshot_source: MemoryContentMetric::from_optional(profile_snapshot),
            legacy_static_block: MemoryContentMetric::from_optional(legacy_static_block),
            legacy_candidate_count,
            pinned_claim_candidate_count,
            injected_ref_count: refs
                .iter()
                .filter(|reference| reference.role == "injected")
                .count(),
            injected_refs_by_origin,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DynamicMemoryContextManifest {
    pub active_recall_present: bool,
    pub active_recall_cached: bool,
    pub active_recall_candidate_count: usize,
    pub active_recall_selected_count: usize,
    pub active_recall_latency_ms: Option<u64>,
    pub active_recall: MemoryContentMetric,
    pub procedure: MemoryContentMetric,
    pub experience_ref_count: usize,
    pub graph_ref_count: usize,
}

impl DynamicMemoryContextManifest {
    pub(crate) fn from_runtime(
        active: Option<&ActiveMemoryRecall>,
        active_suffix: Option<&str>,
        procedure_suffix: Option<&str>,
        experience_ref_count: usize,
        graph_ref_count: usize,
    ) -> Self {
        Self {
            active_recall_present: active.is_some(),
            active_recall_cached: active.is_some_and(|recall| recall.cached),
            active_recall_candidate_count: active.map_or(0, |recall| recall.total_candidates),
            active_recall_selected_count: active
                .and_then(|recall| recall.selected.as_ref())
                .map_or(0, |_| 1),
            active_recall_latency_ms: active.and_then(|recall| recall.latency_ms),
            active_recall: MemoryContentMetric::from_optional(active_suffix),
            procedure: MemoryContentMetric::from_optional(procedure_suffix),
            experience_ref_count,
            graph_ref_count,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemoryContextManifest {
    pub provider: String,
    pub model: String,
    pub round: u32,
    pub rollout_enabled: bool,
    pub shadow_plan: bool,
    pub stable_prompt_fingerprint: String,
    pub static_context: StaticMemoryContextManifest,
    pub dynamic_context: DynamicMemoryContextManifest,
}

impl MemoryContextManifest {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        provider: &str,
        model: &str,
        round: u32,
        rollout_enabled: bool,
        shadow_plan: bool,
        stable_prompt: &str,
        static_context: StaticMemoryContextManifest,
        dynamic_context: DynamicMemoryContextManifest,
    ) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            round,
            rollout_enabled,
            shadow_plan,
            stable_prompt_fingerprint: fingerprint(stable_prompt.as_bytes()),
            static_context,
            dynamic_context,
        }
    }

    pub(crate) fn log(&self) {
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "info",
                "memory",
                "memory::context_manifest",
                &format!(
                    "memory context round {}: {} static refs / {} active candidates",
                    self.round,
                    self.static_context.injected_ref_count,
                    self.dynamic_context.active_recall_candidate_count,
                ),
                serde_json::to_string(self).ok(),
                None,
                None,
            );
        }
    }
}

fn token_estimate(bytes: usize) -> u32 {
    ((bytes + crate::context_compact::CHARS_PER_TOKEN - 1)
        / crate::context_compact::CHARS_PER_TOKEN) as u32
}

fn fingerprint(value: &[u8]) -> String {
    blake3::hash(value).to_hex()[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(origin: &str, role: &str, preview: &str) -> UsedMemoryRef {
        UsedMemoryRef {
            kind: "memory".into(),
            id: "42".into(),
            source_type: "user".into(),
            scope: "agent:ha-main".into(),
            origin: origin.into(),
            role: role.into(),
            preview: preview.into(),
            path: None,
            line: None,
            col: None,
            heading_path: None,
            block_id: None,
            score: None,
            confidence: None,
            salience: None,
        }
    }

    #[test]
    fn manifest_contains_only_metrics_and_hashes() {
        let secret = "user secret preference";
        let refs = vec![reference("static_memory", "injected", secret)];
        let static_context = StaticMemoryContextManifest::from_sources(
            true,
            false,
            true,
            Some(secret),
            None,
            None,
            None,
            None,
            1,
            0,
            &refs,
        );
        let manifest = MemoryContextManifest::new(
            "test",
            "model",
            0,
            false,
            true,
            "stable system",
            static_context,
            DynamicMemoryContextManifest::default(),
        );
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(!json.contains(secret));
        assert!(!json.contains("preview"));
        assert!(json.contains("static_memory"));
        assert!(json.contains("fingerprint"));
    }

    #[test]
    fn dynamic_content_changes_hash_without_entering_static_metric() {
        let first = DynamicMemoryContextManifest::from_runtime(None, Some("recall a"), None, 0, 0);
        let second = DynamicMemoryContextManifest::from_runtime(None, Some("recall b"), None, 0, 0);
        assert_ne!(
            first.active_recall.fingerprint,
            second.active_recall.fingerprint
        );
        assert_eq!(first.procedure.fingerprint, second.procedure.fingerprint);
    }

    #[test]
    fn memory_v2_context_fixture_locks_required_scenarios() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/memory_ux_v2/context_cases.json"
        ))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        let ids = cases
            .iter()
            .filter_map(|case| case["id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for required in [
            "greeting_skips_recall",
            "explicit_personal_preference_recalls_agent_before_global",
            "project_fact_prefers_project_scope",
            "historical_reference_recalls_dynamic_store",
            "project_like_fact_without_project_is_unassigned",
            "incognito_is_zero_memory",
        ] {
            assert!(ids.contains(required), "missing fixture case {required}");
        }
        assert_eq!(ids.len(), cases.len(), "fixture ids must be unique");
    }
}
