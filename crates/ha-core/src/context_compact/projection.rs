//! Request-only projection primitives for Tier 0 / Tier 2 compaction.
//!
//! This module deliberately contains no session IO and does not decide when a
//! projection becomes the active session head.  It provides the pure,
//! provider-neutral half of that boundary: capture text-only result changes
//! from a scratch request, freeze them in an immutable epoch, and replay them
//! against a fresh clone of canonical history.

// The writer/send-path switch is intentionally gated on the durable epoch and
// exact-request work. Keep this pure foundation warning-free until that wiring
// lands as one atomic migration.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde_json::Value;

use super::estimation::{set_tool_result_unit_text, tool_result_units};
use super::types::ToolResultLocator;

const PROJECTION_RENDERER_VERSION: u16 = 1;
static NEXT_PROCESS_EPOCH_ID: AtomicU64 = AtomicU64::new(1);

/// Provider-level shape of a result occurrence.
///
/// The call id is not sufficient on its own: an imported or malformed history
/// could reuse the same id in a different wire shape.  Including the shape in
/// the key makes that case fail closed instead of rewriting an unrelated item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectionResultShape {
    OpenAiChat,
    OpenAiResponses,
    Anthropic,
}

/// Stable, append-safe identity for one tool-result occurrence.
///
/// Provider call ids are required.  Legacy results without one are deliberately
/// not projectable until a durable occurrence sidecar can supply an identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub(crate) struct ProjectionItemKey {
    pub(crate) shape: ProjectionResultShape,
    pub(crate) call_id: String,
}

/// Declarative fidelity selected for a result occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectionActionKind {
    Tier0Omit,
    Tier2Soft,
    Tier2Minimal,
}

impl ProjectionActionKind {
    fn fidelity_rank(self) -> u8 {
        match self {
            Self::Tier2Soft => 2,
            Self::Tier0Omit | Self::Tier2Minimal => 1,
        }
    }
}

/// One frozen request-only replacement.
///
/// `replacement_text` is process-memory request material, not a ResultStore
/// payload and not a persistence format.  Durable epochs should eventually
/// store renderer parameters/result ids rather than copying result bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionAction {
    pub(crate) kind: ProjectionActionKind,
    /// Global result ordinal in the provider-shaped history used to create
    /// this action. It is persisted only as a stable locator/audit aid; replay
    /// still verifies the typed item key and source guard.
    pub(crate) stable_ordinal: usize,
    /// Guards rewinds/imports that reuse a call id with different admitted
    /// content. A stale action is skipped instead of rewriting the new result.
    expected_source_hash: [u8; 32],
    pub(crate) replacement_text: String,
}

/// A set of newly planned actions before it is assigned an epoch id.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectionDraft {
    actions: BTreeMap<ProjectionItemKey, ProjectionAction>,
    pub(crate) skipped_unidentifiable: usize,
    pub(crate) skipped_structural_mismatch: usize,
    pub(crate) skipped_duplicate_key: usize,
}

/// Body-free metadata bridge consumed by the durable epoch writer.
///
/// The replacement itself remains either in the frozen exact request payload
/// or is deterministically rerendered from an authorized ResultStore object.
/// This record only proves which source occurrence was lowered, by which tier,
/// and which exact replacement bytes the live planner selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectionDraftManifestItem {
    pub(crate) key: ProjectionItemKey,
    pub(crate) stable_ordinal: usize,
    pub(crate) kind: ProjectionActionKind,
    pub(crate) source_guard: String,
    pub(crate) replacement_fingerprint: String,
}

impl ProjectionDraftManifestItem {
    pub(crate) fn durable_item_key(&self) -> String {
        let shape = match self.key.shape {
            ProjectionResultShape::OpenAiChat => "openai_chat",
            ProjectionResultShape::OpenAiResponses => "openai_responses",
            ProjectionResultShape::Anthropic => "anthropic",
        };
        format!("{shape}:{}", self.key.call_id)
    }

    pub(crate) const fn action_label(&self) -> &'static str {
        match self.kind {
            ProjectionActionKind::Tier0Omit => "tier0_omit",
            ProjectionActionKind::Tier2Soft => "tier2_soft",
            ProjectionActionKind::Tier2Minimal => "tier2_minimal",
        }
    }
}

impl ProjectionDraft {
    pub(crate) fn action_count(&self) -> usize {
        self.actions.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Convert the exact edits accepted by the capacity-pressure counter into
    /// an immutable projection draft. Unlike a post-hoc history diff, this
    /// preserves whether Tier 2 selected a soft preview or the minimal
    /// envelope and carries the source hash captured before mutation.
    pub(crate) fn from_capacity_pressure_edits(
        edits: &[super::capacity_pressure::CapacityPressureEdit],
    ) -> Self {
        let mut draft = Self::default();
        for edit in edits {
            let Some(key) = item_key(edit.locator, edit.call_id.as_deref()) else {
                draft.skipped_unidentifiable += 1;
                continue;
            };
            if draft.actions.contains_key(&key) {
                draft.skipped_duplicate_key += 1;
                continue;
            }
            draft.actions.insert(
                key,
                ProjectionAction {
                    kind: edit.action,
                    stable_ordinal: edit.result_ordinal,
                    expected_source_hash: edit.expected_source_hash,
                    replacement_text: edit.replacement.clone(),
                },
            );
        }
        draft
    }

    pub(crate) fn manifest_items(&self) -> Vec<ProjectionDraftManifestItem> {
        self.actions
            .iter()
            .map(|(key, action)| ProjectionDraftManifestItem {
                key: key.clone(),
                stable_ordinal: action.stable_ordinal,
                kind: action.kind,
                source_guard: crate::cache_routing::audit_fingerprint(
                    "context-projection-source-v1",
                    blake3::Hash::from(action.expected_source_hash).as_bytes(),
                ),
                replacement_fingerprint: crate::cache_routing::audit_fingerprint(
                    "context-projection-replacement-v1",
                    action.replacement_text.as_bytes(),
                ),
            })
            .collect()
    }
}

/// Small cache label that can travel with an actual projected request snapshot.
///
/// It intentionally makes no durability claim.  `epoch_id` is process-local;
/// the future SessionDB writer must replace it with a durable id before this can
/// be used for crash recovery or exact-request replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectionCacheTag {
    pub(crate) epoch_id: u64,
    pub(crate) renderer_version: u16,
    pub(crate) action_count: usize,
}

/// Immutable, monotonically degrading request projection.
///
/// Ordinary canonical appends do not mutate this action map.  A planner must
/// explicitly create a successor epoch to add new result occurrences or lower
/// the fidelity of an existing one.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectionEpoch {
    epoch_id: u64,
    actions: BTreeMap<ProjectionItemKey, ProjectionAction>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProjectionApplyReport {
    pub(crate) applied: usize,
    pub(crate) already_projected: usize,
    pub(crate) missing: usize,
    pub(crate) duplicate_key: usize,
    pub(crate) source_mismatch: usize,
}

impl ProjectionEpoch {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub(crate) fn action_count(&self) -> usize {
        self.actions.len()
    }

    pub(crate) fn manifest_items(&self) -> Vec<ProjectionDraftManifestItem> {
        ProjectionDraft {
            actions: self.actions.clone(),
            ..ProjectionDraft::default()
        }
        .manifest_items()
    }

    /// Rebuild the complete process-local epoch from canonical history and the
    /// request projection actually selected for the next Provider call.
    ///
    /// This is the compatibility bridge while legacy cheap compaction still
    /// performs Tier 0/Tier 1/Tier 2 in one function and cannot return a typed
    /// action stream. Exact placeholders remain distinguishable; any other
    /// bounded preview is represented as the soft-fidelity action. The frozen
    /// exact request payload remains the byte-level truth.
    pub(crate) fn from_projected_view(
        canonical: &[Value],
        projected: &[Value],
        hard_clear_placeholder: &str,
    ) -> Self {
        let mut draft = ProjectionDraft::default();
        if canonical.len() != projected.len() {
            draft.skipped_structural_mismatch = canonical.len().max(projected.len());
            return Self::empty();
        }
        let duplicate_keys = duplicate_result_keys(canonical)
            .into_iter()
            .chain(duplicate_result_keys(projected))
            .collect::<HashSet<_>>();
        let mut stable_ordinal = 0usize;
        for (canonical_message, projected_message) in canonical.iter().zip(projected) {
            let canonical_units = tool_result_units(canonical_message);
            let projected_units = tool_result_units(projected_message);
            if canonical_units.len() != projected_units.len() {
                draft.skipped_structural_mismatch +=
                    canonical_units.len().max(projected_units.len()).max(1);
                stable_ordinal = stable_ordinal.saturating_add(canonical_units.len());
                continue;
            }
            for (canonical_unit, projected_unit) in canonical_units.iter().zip(&projected_units) {
                let unit_ordinal = stable_ordinal;
                stable_ordinal = stable_ordinal.saturating_add(1);
                if canonical_unit.locator != projected_unit.locator
                    || canonical_unit.call_id != projected_unit.call_id
                    || canonical_unit.text == projected_unit.text
                {
                    continue;
                }
                let Some(key) = item_key(canonical_unit.locator, canonical_unit.call_id.as_deref())
                else {
                    draft.skipped_unidentifiable += 1;
                    continue;
                };
                if duplicate_keys.contains(&key) {
                    draft.skipped_duplicate_key += 1;
                    continue;
                }
                let (Some(source), Some(replacement)) = (
                    canonical_unit.text.as_deref(),
                    projected_unit.text.as_deref(),
                ) else {
                    draft.skipped_structural_mismatch += 1;
                    continue;
                };
                let kind = if replacement == "[Ephemeral tool result cleared]" {
                    ProjectionActionKind::Tier0Omit
                } else if replacement == hard_clear_placeholder {
                    ProjectionActionKind::Tier2Minimal
                } else {
                    ProjectionActionKind::Tier2Soft
                };
                draft.actions.insert(
                    key,
                    ProjectionAction {
                        kind,
                        stable_ordinal: unit_ordinal,
                        expected_source_hash: *blake3::hash(source.as_bytes()).as_bytes(),
                        replacement_text: replacement.to_string(),
                    },
                );
            }
        }
        if draft.actions.is_empty() {
            Self::empty()
        } else {
            Self {
                epoch_id: next_epoch_id(),
                actions: draft.actions,
            }
        }
    }

    pub(crate) fn cache_tag(&self) -> ProjectionCacheTag {
        ProjectionCacheTag {
            epoch_id: self.epoch_id,
            renderer_version: PROJECTION_RENDERER_VERSION,
            action_count: self.actions.len(),
        }
    }

    /// Capture text replacements made by one compaction phase on a scratch
    /// copy. `baseline` must be the current epoch rendered over canonical
    /// history; for the first epoch those views are identical. Structure
    /// changes, missing call ids, duplicate ids, and media-only changes are
    /// skipped rather than guessed.
    pub(crate) fn capture_phase(
        baseline: &[Value],
        scratch: &[Value],
        kind: ProjectionActionKind,
    ) -> ProjectionDraft {
        let mut draft = ProjectionDraft::default();
        if baseline.len() != scratch.len() {
            draft.skipped_structural_mismatch = baseline.len().max(scratch.len());
            return draft;
        }

        let duplicate_keys = duplicate_result_keys(baseline)
            .into_iter()
            .chain(duplicate_result_keys(scratch))
            .collect::<HashSet<_>>();

        let mut stable_ordinal = 0usize;
        for (baseline_message, scratch_message) in baseline.iter().zip(scratch) {
            let baseline_units = tool_result_units(baseline_message);
            let scratch_units = tool_result_units(scratch_message);
            if baseline_units.len() != scratch_units.len() {
                draft.skipped_structural_mismatch +=
                    baseline_units.len().max(scratch_units.len()).max(1);
                stable_ordinal = stable_ordinal.saturating_add(baseline_units.len());
                continue;
            }

            for (baseline_unit, scratch_unit) in baseline_units.iter().zip(&scratch_units) {
                let unit_ordinal = stable_ordinal;
                stable_ordinal = stable_ordinal.saturating_add(1);
                if baseline_unit.locator != scratch_unit.locator
                    || baseline_unit.call_id != scratch_unit.call_id
                {
                    draft.skipped_structural_mismatch += 1;
                    continue;
                }
                if baseline_unit.text == scratch_unit.text {
                    continue;
                }

                let Some(key) = item_key(baseline_unit.locator, baseline_unit.call_id.as_deref())
                else {
                    draft.skipped_unidentifiable += 1;
                    continue;
                };
                if duplicate_keys.contains(&key) {
                    draft.skipped_duplicate_key += 1;
                    continue;
                }
                let Some(replacement_text) = scratch_unit.text.clone() else {
                    draft.skipped_structural_mismatch += 1;
                    continue;
                };
                let Some(source_text) = baseline_unit.text.as_deref() else {
                    draft.skipped_structural_mismatch += 1;
                    continue;
                };
                draft.actions.insert(
                    key,
                    ProjectionAction {
                        kind,
                        stable_ordinal: unit_ordinal,
                        expected_source_hash: *blake3::hash(source_text.as_bytes()).as_bytes(),
                        replacement_text,
                    },
                );
            }
        }

        draft
    }

    /// Create a successor containing the previous actions plus any newly
    /// admitted lower-fidelity actions.  Returns `None` if replay would be
    /// byte-identical to the current epoch.
    pub(crate) fn successor(&self, draft: ProjectionDraft) -> Option<Self> {
        let mut actions = self.actions.clone();
        let mut changed = false;

        for (key, candidate) in draft.actions {
            match actions.get(&key) {
                None => {
                    actions.insert(key, candidate);
                    changed = true;
                }
                Some(current) if candidate.kind.fidelity_rank() < current.kind.fidelity_rank() => {
                    let mut candidate = candidate;
                    // A lower-fidelity draft is captured from the current
                    // projected view. Keep the original canonical guard from
                    // the existing action so the successor still replays from
                    // canonical rather than requiring the previous projection
                    // to be materialized first.
                    candidate.expected_source_hash = current.expected_source_hash;
                    actions.insert(key, candidate);
                    changed = true;
                }
                // Never upgrade an old action or rerender an equal-fidelity
                // action in place.  That keeps an epoch byte-stable as the
                // recent-history boundary moves on ordinary appends.
                Some(_) => {}
            }
        }

        changed.then(|| Self {
            epoch_id: next_epoch_id(),
            actions,
        })
    }

    /// Clone canonical history and apply this epoch to the clone.
    pub(crate) fn project(&self, canonical: &[Value]) -> (Vec<Value>, ProjectionApplyReport) {
        let mut projected = canonical.to_vec();
        let report = self.apply_in_place(&mut projected);
        (projected, report)
    }

    fn apply_in_place(&self, projected: &mut [Value]) -> ProjectionApplyReport {
        let mut report = ProjectionApplyReport::default();
        if self.actions.is_empty() {
            return report;
        }

        let mut locations: HashMap<ProjectionItemKey, Vec<(usize, ToolResultLocator)>> =
            HashMap::new();
        for (message_index, message) in projected.iter().enumerate() {
            for unit in tool_result_units(message) {
                if let Some(key) = item_key(unit.locator, unit.call_id.as_deref()) {
                    locations
                        .entry(key)
                        .or_default()
                        .push((message_index, unit.locator));
                }
            }
        }

        for (key, action) in &self.actions {
            let Some(matches) = locations.get(key) else {
                report.missing += 1;
                continue;
            };
            if matches.len() != 1 {
                report.duplicate_key += 1;
                continue;
            }
            let (message_index, locator) = matches[0];
            let current = tool_result_units(&projected[message_index])
                .into_iter()
                .find(|unit| unit.locator == locator)
                .and_then(|unit| unit.text);
            if current.as_deref() == Some(action.replacement_text.as_str()) {
                report.already_projected += 1;
                continue;
            }
            let Some(current) = current else {
                report.missing += 1;
                continue;
            };
            if blake3::hash(current.as_bytes()).as_bytes() != &action.expected_source_hash {
                report.source_mismatch += 1;
                continue;
            }
            if set_tool_result_unit_text(
                &mut projected[message_index],
                locator,
                &action.replacement_text,
            ) {
                report.applied += 1;
            } else {
                report.missing += 1;
            }
        }

        report
    }
}

fn next_epoch_id() -> u64 {
    NEXT_PROCESS_EPOCH_ID.fetch_add(1, Ordering::Relaxed)
}

fn result_shape(locator: ToolResultLocator) -> ProjectionResultShape {
    match locator {
        ToolResultLocator::OpenAiChatContent => ProjectionResultShape::OpenAiChat,
        ToolResultLocator::OpenAiResponsesOutput => ProjectionResultShape::OpenAiResponses,
        ToolResultLocator::AnthropicBlock(_) => ProjectionResultShape::Anthropic,
    }
}

fn item_key(locator: ToolResultLocator, call_id: Option<&str>) -> Option<ProjectionItemKey> {
    let call_id = call_id?.trim();
    if call_id.is_empty() {
        return None;
    }
    Some(ProjectionItemKey {
        shape: result_shape(locator),
        call_id: call_id.to_string(),
    })
}

fn duplicate_result_keys(messages: &[Value]) -> HashSet<ProjectionItemKey> {
    let mut seen = HashSet::new();
    let mut duplicates = HashSet::new();
    for message in messages {
        for unit in tool_result_units(message) {
            let Some(key) = item_key(unit.locator, unit.call_id.as_deref()) else {
                continue;
            };
            if !seen.insert(key.clone()) {
                duplicates.insert(key);
            }
        }
    }
    duplicates
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn epoch_from_phase(
        canonical: &[Value],
        scratch: &[Value],
        kind: ProjectionActionKind,
    ) -> ProjectionEpoch {
        ProjectionEpoch::empty()
            .successor(ProjectionEpoch::capture_phase(canonical, scratch, kind))
            .expect("phase should create an epoch")
    }

    #[test]
    fn projection_never_mutates_canonical_history() {
        let canonical = vec![json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "full result"
        })];
        let mut scratch = canonical.clone();
        scratch[0]["content"] = json!("[cleared]");
        let epoch = epoch_from_phase(&canonical, &scratch, ProjectionActionKind::Tier0Omit);

        let (projected, report) = epoch.project(&canonical);

        assert_eq!(canonical[0]["content"], "full result");
        assert_eq!(projected[0]["content"], "[cleared]");
        assert_eq!(report.applied, 1);
    }

    #[test]
    fn accepted_capacity_edits_preserve_tier_source_guard_and_replacement_fingerprint() {
        let source = "full old result";
        let replacement = "[cleared]";
        let edits = vec![super::super::capacity_pressure::CapacityPressureEdit {
            result_ordinal: 3,
            call_id: Some("call_3".to_string()),
            locator: ToolResultLocator::OpenAiChatContent,
            action: ProjectionActionKind::Tier0Omit,
            expected_source_hash: *blake3::hash(source.as_bytes()).as_bytes(),
            replacement: replacement.to_string(),
        }];

        let draft = ProjectionDraft::from_capacity_pressure_edits(&edits);
        let manifest = draft.manifest_items();

        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].stable_ordinal, 3);
        assert_eq!(manifest[0].kind, ProjectionActionKind::Tier0Omit);
        assert_eq!(
            manifest[0].source_guard,
            crate::cache_routing::audit_fingerprint(
                "context-projection-source-v1",
                blake3::hash(source.as_bytes()).as_bytes(),
            )
        );
        assert_eq!(
            manifest[0].replacement_fingerprint,
            crate::cache_routing::audit_fingerprint(
                "context-projection-replacement-v1",
                replacement.as_bytes(),
            )
        );
    }

    #[test]
    fn full_request_projection_rebuild_classifies_legacy_and_exact_actions() {
        let canonical = vec![
            json!({"role":"tool","tool_call_id":"eager","content":"full eager"}),
            json!({"role":"tool","tool_call_id":"soft","content":"full soft"}),
            json!({"role":"tool","tool_call_id":"minimal","content":"full minimal"}),
        ];
        let projected = vec![
            json!({"role":"tool","tool_call_id":"eager","content":"[Ephemeral tool result cleared]"}),
            json!({"role":"tool","tool_call_id":"soft","content":"head … tail"}),
            json!({"role":"tool","tool_call_id":"minimal","content":"[cleared]"}),
        ];

        let epoch = ProjectionEpoch::from_projected_view(&canonical, &projected, "[cleared]");
        let manifest = epoch.manifest_items();

        assert_eq!(manifest.len(), 3);
        let by_call_id = manifest
            .iter()
            .map(|item| (item.key.call_id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(by_call_id["eager"].kind, ProjectionActionKind::Tier0Omit);
        assert_eq!(by_call_id["soft"].kind, ProjectionActionKind::Tier2Soft);
        assert_eq!(
            by_call_id["minimal"].kind,
            ProjectionActionKind::Tier2Minimal
        );
        assert_eq!(by_call_id["eager"].stable_ordinal, 0);
        assert_eq!(by_call_id["minimal"].stable_ordinal, 2);
    }

    #[test]
    fn anthropic_parallel_blocks_are_projected_independently() {
        let canonical = vec![json!({
            "role": "user",
            "content": [
                {"type":"tool_result","tool_use_id":"toolu_1","content":"first full"},
                {"type":"text","text":"container text"},
                {
                    "type":"tool_result",
                    "tool_use_id":"toolu_2",
                    "content":[
                        {"type":"text","text":"second full"},
                        {"type":"image","source":{"type":"base64","data":"AA=="}}
                    ]
                }
            ]
        })];
        let mut scratch = canonical.clone();
        scratch[0]["content"][2]["content"][0]["text"] = json!("second preview");
        let epoch = epoch_from_phase(&canonical, &scratch, ProjectionActionKind::Tier2Soft);

        let (projected, report) = epoch.project(&canonical);
        let blocks = projected[0]["content"].as_array().unwrap();

        assert_eq!(report.applied, 1);
        assert_eq!(blocks[0]["content"], "first full");
        assert_eq!(blocks[1]["text"], "container text");
        assert_eq!(blocks[2]["content"][0]["text"], "second preview");
        assert_eq!(blocks[2]["content"][1]["type"], "image");
        assert_eq!(
            canonical[0]["content"][2]["content"][0]["text"],
            "second full"
        );
    }

    #[test]
    fn ordinary_append_does_not_change_old_actions() {
        let canonical = vec![json!({
            "type": "function_call_output",
            "call_id": "fc_old",
            "output": "old full result"
        })];
        let mut scratch = canonical.clone();
        scratch[0]["output"] = json!("old preview");
        let epoch = epoch_from_phase(&canonical, &scratch, ProjectionActionKind::Tier2Soft);
        let original_tag = epoch.cache_tag();

        let mut appended = canonical.clone();
        appended.push(json!({
            "type": "function_call_output",
            "call_id": "fc_new",
            "output": "new full result"
        }));
        let (projected, report) = epoch.project(&appended);

        assert_eq!(epoch.cache_tag(), original_tag);
        assert_eq!(report.applied, 1);
        assert_eq!(projected[0]["output"], "old preview");
        assert_eq!(projected[1]["output"], "new full result");
        assert_eq!(appended[0]["output"], "old full result");
    }

    #[test]
    fn successor_is_monotonic_and_equal_fidelity_is_byte_stable() {
        let canonical = vec![json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "full result"
        })];
        let mut soft = canonical.clone();
        soft[0]["content"] = json!("soft-v1");
        let epoch = epoch_from_phase(&canonical, &soft, ProjectionActionKind::Tier2Soft);

        let mut rerendered_soft = canonical.clone();
        rerendered_soft[0]["content"] = json!("soft-v2");
        let same_fidelity = ProjectionEpoch::capture_phase(
            &canonical,
            &rerendered_soft,
            ProjectionActionKind::Tier2Soft,
        );
        assert!(epoch.successor(same_fidelity).is_none());

        let mut minimal = canonical.clone();
        minimal[0]["content"] = json!("[omitted]");
        let (soft_projected, _) = epoch.project(&canonical);
        let lower = ProjectionEpoch::capture_phase(
            &soft_projected,
            &minimal,
            ProjectionActionKind::Tier2Minimal,
        );
        let successor = epoch.successor(lower).expect("lower fidelity must advance");
        let (projected, _) = successor.project(&canonical);
        assert_eq!(projected[0]["content"], "[omitted]");
    }

    #[test]
    fn duplicate_call_ids_fail_closed() {
        let canonical = vec![
            json!({"role":"tool","tool_call_id":"dup","content":"one"}),
            json!({"role":"tool","tool_call_id":"dup","content":"two"}),
        ];
        let mut scratch = canonical.clone();
        scratch[0]["content"] = json!("changed one");
        scratch[1]["content"] = json!("changed two");

        let draft =
            ProjectionEpoch::capture_phase(&canonical, &scratch, ProjectionActionKind::Tier0Omit);

        assert!(draft.is_empty());
        assert_eq!(draft.skipped_duplicate_key, 2);
    }

    #[test]
    fn reused_call_id_with_changed_source_fails_closed() {
        let canonical = vec![json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "original occurrence"
        })];
        let mut scratch = canonical.clone();
        scratch[0]["content"] = json!("preview");
        let epoch = epoch_from_phase(&canonical, &scratch, ProjectionActionKind::Tier2Soft);

        let reused = vec![json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "different occurrence after import"
        })];
        let (projected, report) = epoch.project(&reused);

        assert_eq!(projected, reused);
        assert_eq!(report.applied, 0);
        assert_eq!(report.source_mismatch, 1);
    }
}
