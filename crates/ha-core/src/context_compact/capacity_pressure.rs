//! Deterministic old-history reclamation for a current-group C0 overflow.
//!
//! This is deliberately separate from the ordinary ratio/TTL-driven Tier 0/2
//! pipeline. The caller supplies the exact complete-request counter and a hard
//! protected suffix containing the current user turn and current tool group.
//! Each accepted edit must strictly reduce that same request upper bound.

use anyhow::{bail, Result};
use serde_json::Value;

use super::config::CompactConfig;
use super::estimation::{
    build_tool_id_to_name_map, get_tool_name_for_result_unit, get_tool_result_unit_text,
    set_tool_result_unit_text, tool_result_units,
};
use super::projection::ProjectionActionKind;
use super::truncation::head_tail_truncate;
use super::types::ToolResultLocator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapacityPressureTier {
    Tier0,
    Tier2,
}

/// One deterministic mutation that can be replayed from the provider-shaped
/// accounting history onto the request-only canonical-shaped projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapacityPressureEdit {
    pub(crate) result_ordinal: usize,
    pub(crate) call_id: Option<String>,
    pub(crate) locator: ToolResultLocator,
    pub(crate) action: ProjectionActionKind,
    pub(crate) expected_source_hash: [u8; 32],
    pub(crate) replacement: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CapacityPressureResult {
    pub(crate) edits: Vec<CapacityPressureEdit>,
    pub(crate) input_upper_before: u64,
    pub(crate) input_upper_after: u64,
    pub(crate) reached_target: bool,
    pub(crate) soft_trimmed: usize,
    pub(crate) hard_cleared: usize,
}

#[derive(Debug, Clone)]
struct Candidate {
    result_ordinal: usize,
    message_index: usize,
    locator: ToolResultLocator,
    call_id: Option<String>,
    tool_name: Option<String>,
    content_bytes: usize,
}

fn candidates_before(
    messages: &[Value],
    protected_start_index: usize,
    config: &CompactConfig,
    tier: CapacityPressureTier,
) -> Vec<Candidate> {
    let id_to_name = build_tool_id_to_name_map(messages);
    let first_genuine_user = messages
        .iter()
        .position(super::estimation::is_user_message)
        .unwrap_or(messages.len());
    let mut ordinal = 0usize;
    let mut candidates = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        for unit in tool_result_units(message) {
            let result_ordinal = ordinal;
            ordinal = ordinal.saturating_add(1);
            if message_index < first_genuine_user || message_index >= protected_start_index {
                continue;
            }
            let tool_name = get_tool_name_for_result_unit(&unit, &id_to_name);
            let eligible = match tier {
                CapacityPressureTier::Tier0 => tool_name
                    .as_deref()
                    .is_some_and(|name| config.is_eager(name)),
                // A protected result may receive a bounded soft preview under
                // hard capacity pressure, but the hard-clear phase below will
                // still exclude it.
                CapacityPressureTier::Tier2 => true,
            };
            if !eligible {
                continue;
            }
            let Some(text) = unit.text else {
                continue;
            };
            candidates.push(Candidate {
                result_ordinal,
                message_index,
                locator: unit.locator,
                call_id: unit.call_id,
                tool_name,
                content_bytes: text.len(),
            });
        }
    }

    if tier == CapacityPressureTier::Tier2 {
        let total_messages = messages.len().max(1);
        candidates.sort_by(|left, right| {
            let score = |candidate: &Candidate| {
                let age = 1.0 - candidate.message_index as f64 / total_messages as f64;
                let size = (candidate.content_bytes as f64 / 100_000.0).min(1.0);
                age * 0.6 + size * 0.4
            };
            score(right)
                .partial_cmp(&score(left))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.result_ordinal.cmp(&right.result_ordinal))
        });
    }
    candidates
}

fn try_replacement<F>(
    messages: &mut [Value],
    candidate: &Candidate,
    replacement: String,
    action: ProjectionActionKind,
    previous_upper: &mut u64,
    count_input_upper: &mut F,
    edits: &mut Vec<CapacityPressureEdit>,
) -> Result<bool>
where
    F: FnMut(&[Value]) -> Result<u64>,
{
    let original = get_tool_result_unit_text(&messages[candidate.message_index], candidate.locator)
        .ok_or_else(|| anyhow::anyhow!("capacity-pressure result text disappeared"))?;
    if replacement.len() >= original.len() || replacement == original {
        return Ok(false);
    }
    if !set_tool_result_unit_text(
        &mut messages[candidate.message_index],
        candidate.locator,
        &replacement,
    ) {
        bail!("capacity-pressure result locator changed");
    }
    let next_upper = match count_input_upper(messages) {
        Ok(count) => count,
        Err(error) => {
            let _ = set_tool_result_unit_text(
                &mut messages[candidate.message_index],
                candidate.locator,
                &original,
            );
            return Err(error);
        }
    };
    if next_upper >= *previous_upper {
        if !set_tool_result_unit_text(
            &mut messages[candidate.message_index],
            candidate.locator,
            &original,
        ) {
            bail!("capacity-pressure result could not be rolled back");
        }
        return Ok(false);
    }
    *previous_upper = next_upper;
    edits.push(CapacityPressureEdit {
        result_ordinal: candidate.result_ordinal,
        call_id: candidate.call_id.clone(),
        locator: candidate.locator,
        action,
        expected_source_hash: *blake3::hash(original.as_bytes()).as_bytes(),
        replacement,
    });
    Ok(true)
}

/// Apply exactly one deterministic recovery tier to old history.
///
/// The target is an input-token upper bound; output reservation and Tier-1
/// safety headroom have already been subtracted by the caller. The counter must
/// cover the same provider roles, dynamic envelopes, tool schemas and history
/// shape used by the pending request.
pub(crate) fn apply_capacity_pressure_tier<F>(
    messages: &mut [Value],
    protected_start_index: usize,
    config: &CompactConfig,
    tier: CapacityPressureTier,
    target_input_upper: u64,
    mut count_input_upper: F,
) -> Result<CapacityPressureResult>
where
    F: FnMut(&[Value]) -> Result<u64>,
{
    let before = count_input_upper(messages)?;
    let mut result = CapacityPressureResult {
        input_upper_before: before,
        input_upper_after: before,
        reached_target: before <= target_input_upper,
        ..CapacityPressureResult::default()
    };
    if result.reached_target {
        return Ok(result);
    }

    let candidates = candidates_before(messages, protected_start_index, config, tier);
    let mut previous_upper = before;
    match tier {
        CapacityPressureTier::Tier0 => {
            for candidate in &candidates {
                let replacement = "[Ephemeral tool result cleared]".to_string();
                if candidate
                    .tool_name
                    .as_deref()
                    .is_some_and(|name| config.is_protected(name))
                    || get_tool_result_unit_text(
                        &messages[candidate.message_index],
                        candidate.locator,
                    )
                    .is_some_and(|text| crate::tools::image_markers::has_valid_image_markers(&text))
                {
                    continue;
                }
                if try_replacement(
                    messages,
                    candidate,
                    replacement,
                    ProjectionActionKind::Tier0Omit,
                    &mut previous_upper,
                    &mut count_input_upper,
                    &mut result.edits,
                )? && previous_upper <= target_input_upper
                {
                    break;
                }
            }
        }
        CapacityPressureTier::Tier2 => {
            let target_chars = config
                .soft_trim_head_chars
                .saturating_add(config.soft_trim_tail_chars)
                .saturating_add(200);
            for candidate in &candidates {
                let Some(original) = get_tool_result_unit_text(
                    &messages[candidate.message_index],
                    candidate.locator,
                ) else {
                    continue;
                };
                if crate::tools::image_markers::has_valid_image_markers(&original) {
                    continue;
                }
                if original.len() <= config.soft_trim_max_chars || original.len() <= target_chars {
                    continue;
                }
                let replacement = head_tail_truncate(&original, target_chars);
                if try_replacement(
                    messages,
                    candidate,
                    replacement,
                    ProjectionActionKind::Tier2Soft,
                    &mut previous_upper,
                    &mut count_input_upper,
                    &mut result.edits,
                )? {
                    result.soft_trimmed = result.soft_trimmed.saturating_add(1);
                    if previous_upper <= target_input_upper {
                        break;
                    }
                }
            }

            if previous_upper > target_input_upper && config.hard_clear_enabled {
                for candidate in &candidates {
                    if candidate
                        .tool_name
                        .as_deref()
                        .is_some_and(|name| config.is_protected(name))
                    {
                        continue;
                    }
                    if get_tool_result_unit_text(
                        &messages[candidate.message_index],
                        candidate.locator,
                    )
                    .is_some_and(|text| crate::tools::image_markers::has_valid_image_markers(&text))
                    {
                        continue;
                    }
                    let replacement = config.hard_clear_placeholder.clone();
                    if try_replacement(
                        messages,
                        candidate,
                        replacement,
                        ProjectionActionKind::Tier2Minimal,
                        &mut previous_upper,
                        &mut count_input_upper,
                        &mut result.edits,
                    )? {
                        result.hard_cleared = result.hard_cleared.saturating_add(1);
                        if previous_upper <= target_input_upper {
                            break;
                        }
                    }
                }
            }
        }
    }

    result.input_upper_after = previous_upper;
    result.reached_target = previous_upper <= target_input_upper;
    Ok(result)
}

/// Replay edits onto a structurally equivalent request-only history. The
/// provider-shaped accounting copy may contain vision transcriptions while the
/// request projection still contains typed image markers, so source text is not
/// compared; ordinal and call identity are the stable correspondence.
pub(crate) fn replay_capacity_pressure_edits(
    messages: &mut [Value],
    edits: &[CapacityPressureEdit],
) -> Result<()> {
    let mut targets = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        for unit in tool_result_units(message) {
            targets.push((message_index, unit.locator, unit.call_id));
        }
    }
    for edit in edits {
        let (message_index, locator, call_id) = targets
            .get(edit.result_ordinal)
            .ok_or_else(|| anyhow::anyhow!("capacity-pressure replay ordinal missing"))?;
        if call_id != &edit.call_id {
            bail!("capacity-pressure replay call identity changed");
        }
        if !set_tool_result_unit_text(&mut messages[*message_index], *locator, &edit.replacement) {
            bail!("capacity-pressure replay locator changed");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pressure_tier_never_changes_protected_suffix_and_stops_at_target() {
        let mut messages = vec![
            json!({"role":"user","content":"older request"}),
            json!({"role":"assistant","tool_calls":[{"id":"old","function":{"name":"grep"}}]}),
            json!({"role":"tool","tool_call_id":"old","content":"x".repeat(20_000)}),
            json!({"role":"user","content":"current request"}),
            json!({"role":"assistant","tool_calls":[{"id":"current","function":{"name":"read"}}]}),
            json!({"role":"tool","tool_call_id":"current","content":"CURRENT"}),
        ];
        let protected = messages[5].clone();
        let result = apply_capacity_pressure_tier(
            &mut messages,
            3,
            &CompactConfig::default(),
            CapacityPressureTier::Tier2,
            2_000,
            |history| Ok(serde_json::to_string(history)?.len() as u64 / 4),
        )
        .unwrap();

        assert!(result.input_upper_after < result.input_upper_before);
        assert_eq!(messages[5], protected);
    }

    #[test]
    fn protected_tool_can_soften_but_cannot_hard_clear() {
        let mut config = CompactConfig::default();
        config
            .tool_policies
            .insert("web_fetch".to_string(), "protect".to_string());
        let mut messages = vec![
            json!({"role":"user","content":"older request"}),
            json!({"role":"assistant","tool_calls":[{"id":"old","function":{"name":"web_fetch"}}]}),
            json!({"role":"tool","tool_call_id":"old","content":"x".repeat(20_000)}),
            json!({"role":"user","content":"current request"}),
        ];
        let result = apply_capacity_pressure_tier(
            &mut messages,
            3,
            &config,
            CapacityPressureTier::Tier2,
            0,
            |history| Ok(serde_json::to_string(history)?.len() as u64),
        )
        .unwrap();

        assert_eq!(result.soft_trimmed, 1);
        assert_eq!(result.hard_cleared, 0);
        assert_ne!(messages[2]["content"], config.hard_clear_placeholder);
    }
}
