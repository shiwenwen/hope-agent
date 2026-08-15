//! Pure Tier 1 admission planning for one current tool-result group.
//!
//! This module intentionally has no storage, provider, or chat-loop dependency.
//! Candidate `0` of every result is the caller-rendered cheapest protocol-legal
//! shape (C0). The planner never removes or reorders results: it only selects a
//! richer candidate index for each result in the model's original call order.
//!
//! The live tool loop renders provider-valid candidates first and sends the
//! exact in-memory history snapshot that passed this planner. Durable exact
//! request replay remains a separate request-plan/dispatch-WAL layer.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;

/// Terminal, provider-independent proof that the current tool-result group
/// cannot fit even when every result is rendered at its cheapest legal C0.
///
/// The live orchestrator may first reclaim *older* history and re-run the
/// planner. If every configured Tier 0/2/3 recovery step is exhausted, this
/// concrete error crosses the failover boundary unchanged so it cannot be
/// mistaken for an opaque Provider/network failure and retried blindly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CurrentToolGroupEnvelopeOverflowError {
    pub capacity: RequestCapacityCount,
    pub context_window: u64,
    pub safety_headroom: u64,
}

impl fmt::Display for CurrentToolGroupEnvelopeOverflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "current tool group C0 request does not fit: total_upper={}, safety={}, window={}",
            self.capacity.total_upper_bound(),
            self.safety_headroom,
            self.context_window
        )
    }
}

impl std::error::Error for CurrentToolGroupEnvelopeOverflowError {}

/// Token bounds for one already-rendered result candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateTokenCount {
    pub lower_bound: u64,
    pub estimated: u64,
    pub upper_bound: u64,
}

impl CandidateTokenCount {
    pub const fn new(lower_bound: u64, estimated: u64, upper_bound: u64) -> Self {
        Self {
            lower_bound,
            estimated,
            upper_bound,
        }
    }

    fn is_ordered(self) -> bool {
        self.lower_bound <= self.estimated && self.estimated <= self.upper_bound
    }
}

/// One provider-valid, immutable projection candidate.
///
/// `stable_id` identifies the exact rendered payload in the caller's candidate
/// table. `semantic_rank=0` is C0. Later candidates must have strictly increasing
/// ranks and non-decreasing estimated cost. Byte metadata is measured before
/// tokenization and keeps the planner from treating an already-exact short
/// result as upgradeable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionCandidate {
    pub stable_id: String,
    pub semantic_rank: u8,
    pub kind: AdmissionCandidateKind,
    /// Bytes in the complete effective result before projection.
    pub source_bytes: usize,
    /// Bytes in this exact provider-valid rendered projection.
    pub rendered_bytes: usize,
    pub tokens: CandidateTokenCount,
}

/// Whether a rendered candidate contains the complete effective result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionCandidateKind {
    /// A bounded omission/preview shape. Its footer may make `rendered_bytes`
    /// larger than `source_bytes`, so the planner does not compare those fields.
    OmissionPreview,
    /// The complete effective result with no omitted bytes.
    Exact,
}

/// Typed importance used only for deterministic upgrade fairness.
///
/// It cannot add, remove, move, or merge protocol results. In particular, an
/// error result may receive a richer preview earlier while retaining the exact
/// same call pairing and original position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultAdmissionPriority {
    ErrorOrTimeout,
    ActionReceipt,
    StructuredRead,
    Snapshot,
    Unknown,
}

impl ResultAdmissionPriority {
    /// Stable integer weights avoid floating-point and platform-dependent ties.
    pub const fn weight_millis(self) -> u32 {
        match self {
            Self::ErrorOrTimeout => 3_000,
            Self::ActionReceipt => 2_000,
            Self::StructuredRead => 1_500,
            Self::Snapshot | Self::Unknown => 1_000,
        }
    }
}

/// Candidates for one result occurrence.
///
/// The slice order of `ResultCandidateSet` values is the model's original call
/// order and must agree with the contiguous `model_call_ordinal` values.
/// `call_id` is copied through unchanged; `result_key` is the unique occurrence
/// identity and may include retry identity when call ids repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultCandidateSet {
    pub result_key: String,
    pub call_id: String,
    pub model_call_ordinal: usize,
    pub priority: ResultAdmissionPriority,
    pub candidates: Vec<AdmissionCandidate>,
}

impl ResultCandidateSet {
    /// Build the only legal candidate table for a short, complete result.
    ///
    /// An exact C0 cannot have a richer semantic representation, so validation
    /// rejects any later candidate appended to this table.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn singleton_exact(
        result_key: impl Into<String>,
        call_id: impl Into<String>,
        model_call_ordinal: usize,
        priority: ResultAdmissionPriority,
        stable_id: impl Into<String>,
        source_bytes: usize,
        tokens: CandidateTokenCount,
    ) -> Self {
        Self {
            result_key: result_key.into(),
            call_id: call_id.into(),
            model_call_ordinal,
            priority,
            candidates: vec![AdmissionCandidate {
                stable_id: stable_id.into(),
                semantic_rank: 0,
                kind: AdmissionCandidateKind::Exact,
                source_bytes,
                rendered_bytes: source_bytes,
                tokens,
            }],
        }
    }
}

/// Capacity count for the final complete provider request.
///
/// The evaluator reports input and reserved output separately. `total_upper`
/// adds the reservation exactly once, so callers must not add it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestCapacityCount {
    pub input_upper_bound: u64,
    pub reserved_output: u64,
}

impl RequestCapacityCount {
    pub const fn new(input_upper_bound: u64, reserved_output: u64) -> Self {
        Self {
            input_upper_bound,
            reserved_output,
        }
    }

    pub fn total_upper_bound(self) -> u64 {
        self.input_upper_bound.saturating_add(self.reserved_output)
    }

    pub fn fits(self, context_window: u64, safety_headroom: u64) -> bool {
        self.total_upper_bound()
            .checked_add(safety_headroom)
            .is_some_and(|total| total <= context_window)
    }
}

/// Policy budgets apply only to upgrades above each result's C0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupAdmissionBudget {
    pub context_window: u64,
    pub safety_headroom: u64,
    pub group_upgrade_budget: u64,
    pub per_result_upgrade_ceiling: u64,
}

impl GroupAdmissionBudget {
    /// Tier 1 design defaults. `base_total_estimated` already includes every C0
    /// result and the output reservation.
    pub fn tier1_defaults(
        context_window: u64,
        base_total_estimated: u64,
    ) -> Result<Self, InvalidContextWindow> {
        if context_window == 0 {
            return Err(InvalidContextWindow);
        }

        let one_percent = context_window / 100;
        let safety_headroom = one_percent.clamp(512, 2_000).min(context_window / 10);
        let upgrade_headroom = context_window
            .saturating_sub(safety_headroom)
            .saturating_sub(base_total_estimated);

        Ok(Self {
            context_window,
            safety_headroom,
            group_upgrade_budget: upgrade_headroom.min(context_window / 4),
            per_result_upgrade_ceiling: (context_window / 10).min(8_000),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidContextWindow;

impl fmt::Display for InvalidContextWindow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Tier 1 admission requires a non-zero context window")
    }
}

impl std::error::Error for InvalidContextWindow {}

/// The provider integration supplies the exact, fully expanded request count.
///
/// `selected_candidate_indices` is parallel to the original result slice. The
/// caller can use it to assemble provider messages, apply its visual bridge, and
/// count the final request upper bound. Provider-count unavailability should be
/// handled by the caller's conservative local counter before returning here.
pub trait FinalRequestUpperBoundEvaluator {
    type Error;

    fn evaluate(
        &mut self,
        selected_candidate_indices: &[usize],
    ) -> Result<RequestCapacityCount, Self::Error>;
}

impl<F, E> FinalRequestUpperBoundEvaluator for F
where
    F: FnMut(&[usize]) -> Result<RequestCapacityCount, E>,
{
    type Error = E;

    fn evaluate(
        &mut self,
        selected_candidate_indices: &[usize],
    ) -> Result<RequestCapacityCount, Self::Error> {
        self(selected_candidate_indices)
    }
}

/// Uncertainty-aware delta between adjacent candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateDelta {
    pub lower_bound: u64,
    pub estimated: u64,
    pub upper_bound: u64,
}

impl CandidateDelta {
    fn between(from: CandidateTokenCount, to: CandidateTokenCount) -> Self {
        Self {
            lower_bound: to.lower_bound.saturating_sub(from.upper_bound),
            estimated: to.estimated.saturating_sub(from.estimated),
            upper_bound: to.upper_bound.saturating_sub(from.lower_bound),
        }
    }
}

/// One budget-independent step in the canonical weighted upgrade sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalUpgradeStep {
    pub result_index: usize,
    pub from_candidate_index: usize,
    pub to_candidate_index: usize,
    pub delta: CandidateDelta,
    pub result_spent_after: u64,
    pub weight_millis: u32,
}

/// Final selection for one result. Entries remain in original call order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultAdmissionSelection {
    pub result_key: String,
    pub call_id: String,
    pub original_call_order: usize,
    pub candidate_index: usize,
    pub candidate_stable_id: String,
    pub semantic_rank: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupAdmissionPlan {
    pub selections: Vec<ResultAdmissionSelection>,
    pub canonical_upgrades: Vec<CanonicalUpgradeStep>,
    /// Longest canonical prefix admitted by the estimated group policy budget.
    pub policy_prefix_len: usize,
    /// Longest prefix that also passed the final complete-request upper bound.
    pub selected_prefix_len: usize,
    pub group_upgrade_estimated: u64,
    pub final_capacity: RequestCapacityCount,
    pub final_evaluations: usize,
}

#[derive(Debug)]
pub enum GroupAdmissionError<E> {
    InvalidContextWindow,
    EmptyGroup,
    EmptyResultKey {
        result_index: usize,
    },
    EmptyCallId {
        result_index: usize,
    },
    DuplicateResultKey {
        result_key: String,
    },
    InvalidModelCallOrder {
        result_index: usize,
        expected_ordinal: usize,
        actual_ordinal: usize,
    },
    MissingC0 {
        result_index: usize,
    },
    EmptyCandidateId {
        result_index: usize,
        candidate_index: usize,
    },
    InvalidCandidateBounds {
        result_index: usize,
        candidate_index: usize,
    },
    InvalidCandidateOrder {
        result_index: usize,
        candidate_index: usize,
    },
    DuplicateCandidateId {
        result_index: usize,
        stable_id: String,
    },
    InconsistentCandidateSourceBytes {
        result_index: usize,
        candidate_index: usize,
        expected_source_bytes: usize,
        actual_source_bytes: usize,
    },
    InvalidExactCandidate {
        result_index: usize,
        candidate_index: usize,
    },
    FinalCapacityEvaluationFailed {
        attempted_prefix_len: usize,
        source: E,
    },
    CurrentToolGroupEnvelopeOverflow {
        capacity: RequestCapacityCount,
        context_window: u64,
        safety_headroom: u64,
    },
}

impl<E: fmt::Display> fmt::Display for GroupAdmissionError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContextWindow => formatter.write_str("invalid zero context window"),
            Self::EmptyGroup => formatter.write_str("tool result group is empty"),
            Self::EmptyResultKey { result_index } => {
                write!(formatter, "result {result_index} has an empty result key")
            }
            Self::EmptyCallId { result_index } => {
                write!(formatter, "result {result_index} has an empty call id")
            }
            Self::DuplicateResultKey { result_key } => {
                write!(formatter, "duplicate result key: {result_key}")
            }
            Self::InvalidModelCallOrder {
                result_index,
                expected_ordinal,
                actual_ordinal,
            } => write!(
                formatter,
                "result {result_index} has model call ordinal {actual_ordinal}, expected {expected_ordinal}"
            ),
            Self::MissingC0 { result_index } => {
                write!(formatter, "result {result_index} has no C0 candidate")
            }
            Self::EmptyCandidateId {
                result_index,
                candidate_index,
            } => write!(
                formatter,
                "result {result_index} candidate {candidate_index} has an empty stable id"
            ),
            Self::InvalidCandidateBounds {
                result_index,
                candidate_index,
            } => write!(
                formatter,
                "result {result_index} candidate {candidate_index} has invalid token bounds"
            ),
            Self::InvalidCandidateOrder {
                result_index,
                candidate_index,
            } => write!(
                formatter,
                "result {result_index} candidate {candidate_index} is not a monotone upgrade"
            ),
            Self::DuplicateCandidateId {
                result_index,
                stable_id,
            } => write!(
                formatter,
                "result {result_index} has duplicate candidate id {stable_id}"
            ),
            Self::InconsistentCandidateSourceBytes {
                result_index,
                candidate_index,
                expected_source_bytes,
                actual_source_bytes,
            } => write!(
                formatter,
                "result {result_index} candidate {candidate_index} has source bytes {actual_source_bytes}, expected {expected_source_bytes}"
            ),
            Self::InvalidExactCandidate {
                result_index,
                candidate_index,
            } => write!(
                formatter,
                "result {result_index} candidate {candidate_index} violates the exact-result contract"
            ),
            Self::FinalCapacityEvaluationFailed {
                attempted_prefix_len,
                source,
            } => write!(
                formatter,
                "final request capacity evaluation failed at prefix {attempted_prefix_len}: {source}"
            ),
            Self::CurrentToolGroupEnvelopeOverflow {
                capacity,
                context_window,
                safety_headroom,
            } => write!(
                formatter,
                "current tool group C0 request does not fit: total_upper={}, safety={}, window={}",
                capacity.total_upper_bound(),
                safety_headroom,
                context_window
            ),
        }
    }
}

impl<E> std::error::Error for GroupAdmissionError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FinalCapacityEvaluationFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct UpgradeChoice {
    result_index: usize,
    from_candidate_index: usize,
    to_candidate_index: usize,
    delta: CandidateDelta,
    result_spent_after: u64,
    weight_millis: u32,
}

fn compare_upgrade_choice(left: &UpgradeChoice, right: &UpgradeChoice) -> Ordering {
    // Compare water levels exactly: left_spent/left_weight vs
    // right_spent/right_weight. u64*u32 is bounded by u128.
    let left_cross = u128::from(left.result_spent_after) * u128::from(right.weight_millis);
    let right_cross = u128::from(right.result_spent_after) * u128::from(left.weight_millis);
    left_cross
        .cmp(&right_cross)
        .then_with(|| left.result_index.cmp(&right.result_index))
        .then_with(|| left.to_candidate_index.cmp(&right.to_candidate_index))
}

fn validate_inputs<E>(
    results: &[ResultCandidateSet],
    budget: GroupAdmissionBudget,
) -> Result<(), GroupAdmissionError<E>> {
    if budget.context_window == 0 {
        return Err(GroupAdmissionError::InvalidContextWindow);
    }
    if results.is_empty() {
        return Err(GroupAdmissionError::EmptyGroup);
    }

    let mut result_keys = HashSet::with_capacity(results.len());
    for (result_index, result) in results.iter().enumerate() {
        if result.result_key.is_empty() {
            return Err(GroupAdmissionError::EmptyResultKey { result_index });
        }
        if result.call_id.is_empty() {
            return Err(GroupAdmissionError::EmptyCallId { result_index });
        }
        if !result_keys.insert(result.result_key.as_str()) {
            return Err(GroupAdmissionError::DuplicateResultKey {
                result_key: result.result_key.clone(),
            });
        }
        if result.model_call_ordinal != result_index {
            return Err(GroupAdmissionError::InvalidModelCallOrder {
                result_index,
                expected_ordinal: result_index,
                actual_ordinal: result.model_call_ordinal,
            });
        }
        let Some(first) = result.candidates.first() else {
            return Err(GroupAdmissionError::MissingC0 { result_index });
        };
        if first.semantic_rank != 0 {
            return Err(GroupAdmissionError::MissingC0 { result_index });
        }

        let mut candidate_ids = HashSet::with_capacity(result.candidates.len());
        let mut previous_rank = None;
        let mut previous_estimate = None;
        let source_bytes = first.source_bytes;
        for (candidate_index, candidate) in result.candidates.iter().enumerate() {
            if candidate.stable_id.trim().is_empty() {
                return Err(GroupAdmissionError::EmptyCandidateId {
                    result_index,
                    candidate_index,
                });
            }
            if !candidate.tokens.is_ordered() {
                return Err(GroupAdmissionError::InvalidCandidateBounds {
                    result_index,
                    candidate_index,
                });
            }
            if !candidate_ids.insert(candidate.stable_id.as_str()) {
                return Err(GroupAdmissionError::DuplicateCandidateId {
                    result_index,
                    stable_id: candidate.stable_id.clone(),
                });
            }
            if candidate.source_bytes != source_bytes {
                return Err(GroupAdmissionError::InconsistentCandidateSourceBytes {
                    result_index,
                    candidate_index,
                    expected_source_bytes: source_bytes,
                    actual_source_bytes: candidate.source_bytes,
                });
            }
            if candidate.kind == AdmissionCandidateKind::Exact
                && (candidate.rendered_bytes != candidate.source_bytes
                    || candidate_index + 1 != result.candidates.len()
                    || (candidate_index == 0 && result.candidates.len() != 1))
            {
                return Err(GroupAdmissionError::InvalidExactCandidate {
                    result_index,
                    candidate_index,
                });
            }
            if previous_rank.is_some_and(|rank| candidate.semantic_rank <= rank)
                || previous_estimate.is_some_and(|estimate| candidate.tokens.estimated < estimate)
            {
                return Err(GroupAdmissionError::InvalidCandidateOrder {
                    result_index,
                    candidate_index,
                });
            }
            previous_rank = Some(candidate.semantic_rank);
            previous_estimate = Some(candidate.tokens.estimated);
        }
    }
    Ok(())
}

fn canonical_upgrade_sequence(
    results: &[ResultCandidateSet],
    per_result_upgrade_ceiling: u64,
) -> Vec<CanonicalUpgradeStep> {
    let mut selected = vec![0usize; results.len()];
    let mut spent = vec![0u64; results.len()];
    let mut upgrades = Vec::new();

    loop {
        let mut best: Option<UpgradeChoice> = None;
        for (result_index, result) in results.iter().enumerate() {
            let from_candidate_index = selected[result_index];
            let to_candidate_index = from_candidate_index + 1;
            let Some(to) = result.candidates.get(to_candidate_index) else {
                continue;
            };
            let from = &result.candidates[from_candidate_index];
            let delta = CandidateDelta::between(from.tokens, to.tokens);
            let result_spent_after = spent[result_index].saturating_add(delta.estimated);
            if result_spent_after > per_result_upgrade_ceiling {
                continue;
            }
            let choice = UpgradeChoice {
                result_index,
                from_candidate_index,
                to_candidate_index,
                delta,
                result_spent_after,
                weight_millis: result.priority.weight_millis(),
            };
            if best
                .as_ref()
                .is_none_or(|current| compare_upgrade_choice(&choice, current).is_lt())
            {
                best = Some(choice);
            }
        }

        let Some(choice) = best else {
            break;
        };
        selected[choice.result_index] = choice.to_candidate_index;
        spent[choice.result_index] = choice.result_spent_after;
        upgrades.push(CanonicalUpgradeStep {
            result_index: choice.result_index,
            from_candidate_index: choice.from_candidate_index,
            to_candidate_index: choice.to_candidate_index,
            delta: choice.delta,
            result_spent_after: choice.result_spent_after,
            weight_millis: choice.weight_millis,
        });
    }

    upgrades
}

fn policy_prefix_len(upgrades: &[CanonicalUpgradeStep], group_budget: u64) -> usize {
    let mut spent = 0u64;
    let mut prefix_len = 0usize;
    for upgrade in upgrades {
        let next_spent = spent.saturating_add(upgrade.delta.estimated);
        if next_spent > group_budget {
            break;
        }
        spent = next_spent;
        prefix_len += 1;
    }
    prefix_len
}

fn selections_for_prefix(
    result_count: usize,
    upgrades: &[CanonicalUpgradeStep],
    prefix_len: usize,
) -> Vec<usize> {
    let mut selected = vec![0usize; result_count];
    for upgrade in upgrades.iter().take(prefix_len) {
        debug_assert_eq!(
            selected[upgrade.result_index], upgrade.from_candidate_index,
            "canonical upgrade sequence must be contiguous per result"
        );
        selected[upgrade.result_index] = upgrade.to_candidate_index;
    }
    selected
}

fn estimated_spend(upgrades: &[CanonicalUpgradeStep], prefix_len: usize) -> u64 {
    upgrades.iter().take(prefix_len).fold(0u64, |spent, step| {
        spent.saturating_add(step.delta.estimated)
    })
}

/// Plan one current result group and verify the final complete provider request.
///
/// The group budget selects a deterministic prefix of a budget-independent
/// canonical upgrade sequence. If the final provider shape is too large, the
/// planner removes upgrades strictly in reverse canonical order. Therefore the
/// returned plan is the longest safe prefix, and increasing only the group
/// budget can never make any result select a lower candidate.
pub fn plan_group_admission<Evaluator>(
    results: &[ResultCandidateSet],
    budget: GroupAdmissionBudget,
    evaluator: &mut Evaluator,
) -> Result<GroupAdmissionPlan, GroupAdmissionError<Evaluator::Error>>
where
    Evaluator: FinalRequestUpperBoundEvaluator,
{
    validate_inputs(results, budget)?;
    let canonical_upgrades = canonical_upgrade_sequence(results, budget.per_result_upgrade_ceiling);
    let policy_prefix_len = policy_prefix_len(&canonical_upgrades, budget.group_upgrade_budget);
    let mut selected_prefix_len = policy_prefix_len;
    let mut selected =
        selections_for_prefix(results.len(), &canonical_upgrades, selected_prefix_len);
    let mut final_evaluations = 0usize;

    let final_capacity = loop {
        final_evaluations += 1;
        let capacity = evaluator.evaluate(&selected).map_err(|source| {
            GroupAdmissionError::FinalCapacityEvaluationFailed {
                attempted_prefix_len: selected_prefix_len,
                source,
            }
        })?;
        if capacity.fits(budget.context_window, budget.safety_headroom) {
            break capacity;
        }
        if selected_prefix_len == 0 {
            return Err(GroupAdmissionError::CurrentToolGroupEnvelopeOverflow {
                capacity,
                context_window: budget.context_window,
                safety_headroom: budget.safety_headroom,
            });
        }

        selected_prefix_len -= 1;
        let reverted = &canonical_upgrades[selected_prefix_len];
        debug_assert_eq!(
            selected[reverted.result_index], reverted.to_candidate_index,
            "final fallback must unwind canonical upgrades in reverse order"
        );
        selected[reverted.result_index] = reverted.from_candidate_index;
    };

    let selections = results
        .iter()
        .zip(selected)
        .map(|(result, candidate_index)| {
            let candidate = &result.candidates[candidate_index];
            ResultAdmissionSelection {
                result_key: result.result_key.clone(),
                call_id: result.call_id.clone(),
                original_call_order: result.model_call_ordinal,
                candidate_index,
                candidate_stable_id: candidate.stable_id.clone(),
                semantic_rank: candidate.semantic_rank,
            }
        })
        .collect();

    let group_upgrade_estimated = estimated_spend(&canonical_upgrades, selected_prefix_len);
    Ok(GroupAdmissionPlan {
        selections,
        canonical_upgrades,
        policy_prefix_len,
        selected_prefix_len,
        group_upgrade_estimated,
        final_capacity,
        final_evaluations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, rank: u8, tokens: u64) -> AdmissionCandidate {
        AdmissionCandidate {
            stable_id: id.to_string(),
            semantic_rank: rank,
            kind: AdmissionCandidateKind::OmissionPreview,
            source_bytes: 1_024,
            rendered_bytes: tokens as usize,
            tokens: CandidateTokenCount::new(tokens, tokens, tokens),
        }
    }

    fn result(
        key: &str,
        call_id: &str,
        priority: ResultAdmissionPriority,
        token_levels: &[u64],
    ) -> ResultCandidateSet {
        ResultCandidateSet {
            result_key: key.to_string(),
            call_id: call_id.to_string(),
            model_call_ordinal: 0,
            priority,
            candidates: token_levels
                .iter()
                .enumerate()
                .map(|(index, tokens)| candidate(&format!("c{index}"), index as u8, *tokens))
                .collect(),
        }
    }

    fn ordered(mut results: Vec<ResultCandidateSet>) -> Vec<ResultCandidateSet> {
        for (ordinal, result) in results.iter_mut().enumerate() {
            result.model_call_ordinal = ordinal;
        }
        results
    }

    fn budget(group_upgrade_budget: u64) -> GroupAdmissionBudget {
        GroupAdmissionBudget {
            context_window: 10_000,
            safety_headroom: 100,
            group_upgrade_budget,
            per_result_upgrade_ceiling: 1_000,
        }
    }

    fn always_fits(selected: &[usize]) -> Result<RequestCapacityCount, std::convert::Infallible> {
        Ok(RequestCapacityCount::new(
            100 + selected.iter().map(|value| *value as u64).sum::<u64>(),
            100,
        ))
    }

    #[test]
    fn equal_weight_ties_follow_original_model_call_order() {
        let results = ordered(vec![
            result(
                "r1",
                "call-first",
                ResultAdmissionPriority::Unknown,
                &[0, 10],
            ),
            result(
                "r2",
                "call-second",
                ResultAdmissionPriority::Unknown,
                &[0, 10],
            ),
        ]);
        let plan = plan_group_admission(&results, budget(10), &mut always_fits).unwrap();

        assert_eq!(plan.canonical_upgrades[0].result_index, 0);
        assert_eq!(plan.selections[0].candidate_index, 1);
        assert_eq!(plan.selections[1].candidate_index, 0);
    }

    #[test]
    fn typed_error_weight_changes_upgrade_order_but_not_protocol_pairing() {
        let results = ordered(vec![
            result(
                "r1",
                "call-snapshot",
                ResultAdmissionPriority::Snapshot,
                &[0, 9],
            ),
            result(
                "r2",
                "call-error",
                ResultAdmissionPriority::ErrorOrTimeout,
                &[0, 9],
            ),
        ]);
        let plan = plan_group_admission(&results, budget(9), &mut always_fits).unwrap();

        assert_eq!(plan.canonical_upgrades[0].result_index, 1);
        assert_eq!(
            plan.selections
                .iter()
                .map(|selection| selection.call_id.as_str())
                .collect::<Vec<_>>(),
            vec!["call-snapshot", "call-error"]
        );
        assert_eq!(plan.selections.len(), results.len());
    }

    #[test]
    fn group_budget_takes_a_prefix_and_never_skips_a_step_that_does_not_fit() {
        let results = ordered(vec![
            result(
                "r-error",
                "call-error",
                ResultAdmissionPriority::ErrorOrTimeout,
                &[0, 6],
            ),
            result(
                "r-snapshot",
                "call-snapshot",
                ResultAdmissionPriority::Snapshot,
                &[0, 2],
            ),
        ]);
        let plan = plan_group_admission(&results, budget(5), &mut always_fits).unwrap();

        assert_eq!(plan.canonical_upgrades[0].result_index, 0);
        assert_eq!(plan.policy_prefix_len, 0);
        assert_eq!(plan.selections[0].candidate_index, 0);
        assert_eq!(plan.selections[1].candidate_index, 0);
    }

    #[test]
    fn increasing_group_budget_never_downgrades_any_result() {
        let results = ordered(vec![
            result(
                "r1",
                "call-1",
                ResultAdmissionPriority::StructuredRead,
                &[0, 4, 9],
            ),
            result("r2", "call-2", ResultAdmissionPriority::Unknown, &[0, 3, 8]),
        ]);

        let low = plan_group_admission(&results, budget(4), &mut always_fits).unwrap();
        let high = plan_group_admission(&results, budget(20), &mut always_fits).unwrap();
        for (low, high) in low.selections.iter().zip(&high.selections) {
            assert!(high.candidate_index >= low.candidate_index);
        }
    }

    #[test]
    fn per_result_ceiling_filters_upgrades_before_group_budget_is_read() {
        let results = ordered(vec![result(
            "r1",
            "call-1",
            ResultAdmissionPriority::ErrorOrTimeout,
            &[0, 6, 7],
        )]);
        let mut constrained = budget(100);
        constrained.per_result_upgrade_ceiling = 5;

        let plan = plan_group_admission(&results, constrained, &mut always_fits).unwrap();
        assert!(plan.canonical_upgrades.is_empty());
        assert_eq!(plan.selections[0].candidate_index, 0);
    }

    #[test]
    fn provider_final_upper_bound_downgrades_to_longest_safe_prefix() {
        let results = ordered(vec![
            result(
                "r1",
                "call-1",
                ResultAdmissionPriority::Unknown,
                &[0, 10, 20],
            ),
            result(
                "r2",
                "call-2",
                ResultAdmissionPriority::Unknown,
                &[0, 10, 20],
            ),
        ]);
        let mut constrained = budget(100);
        constrained.context_window = 125;
        constrained.safety_headroom = 5;
        let levels = [[0u64, 10, 20], [0u64, 10, 20]];
        let mut final_evaluator = |selected: &[usize]| -> Result<_, &'static str> {
            let selected_cost = selected
                .iter()
                .enumerate()
                .map(|(result_index, candidate_index)| levels[result_index][*candidate_index])
                .sum::<u64>();
            Ok(RequestCapacityCount::new(100 + selected_cost, 0))
        };

        let plan = plan_group_admission(&results, constrained, &mut final_evaluator).unwrap();

        assert_eq!(plan.policy_prefix_len, 4);
        assert_eq!(plan.selected_prefix_len, 2);
        assert_eq!(plan.selections[0].candidate_index, 1);
        assert_eq!(plan.selections[1].candidate_index, 1);
        assert_eq!(plan.final_capacity.total_upper_bound(), 120);
        assert_eq!(plan.final_evaluations, 3);
    }

    #[test]
    fn c0_overflow_is_typed_and_never_drops_a_result_pair() {
        let results = ordered(vec![
            result("r1", "call-1", ResultAdmissionPriority::Unknown, &[0, 10]),
            result("r2", "call-2", ResultAdmissionPriority::Unknown, &[0, 10]),
        ]);
        let mut constrained = budget(0);
        constrained.context_window = 100;
        constrained.safety_headroom = 5;
        let mut evaluator = |_selected: &[usize]| -> Result<_, &'static str> {
            Ok(RequestCapacityCount::new(96, 0))
        };

        let error = plan_group_admission(&results, constrained, &mut evaluator).unwrap_err();
        assert!(matches!(
            error,
            GroupAdmissionError::CurrentToolGroupEnvelopeOverflow { .. }
        ));
    }

    #[test]
    fn non_zero_c0_is_not_charged_against_the_upgrade_budget() {
        let results = ordered(vec![result(
            "r1",
            "call-1",
            ResultAdmissionPriority::Unknown,
            &[7, 12],
        )]);

        let plan = plan_group_admission(&results, budget(5), &mut always_fits).unwrap();

        assert_eq!(plan.canonical_upgrades[0].delta.estimated, 5);
        assert_eq!(plan.group_upgrade_estimated, 5);
        assert_eq!(plan.selections[0].candidate_index, 1);
    }

    #[test]
    fn complete_request_capacity_accepts_window_boundary_and_rejects_plus_one() {
        assert!(RequestCapacityCount::new(90, 5).fits(100, 5));
        assert!(!RequestCapacityCount::new(91, 5).fits(100, 5));
    }

    #[test]
    fn final_capacity_charges_provider_fixed_dynamic_history_tools_and_output_once() {
        let results = vec![ResultCandidateSet::singleton_exact(
            "r1",
            "call-1",
            0,
            ResultAdmissionPriority::StructuredRead,
            "c0",
            4,
            CandidateTokenCount::new(1, 1, 1),
        )];
        let stable = 100u64;
        let dynamic = 200u64;
        let history_with_c0 = 300u64;
        let tools = 400u64;
        let max_output_tokens = 1_000u64;
        let safety = 512u64;
        let exact_window = stable + dynamic + history_with_c0 + tools + max_output_tokens + safety;
        let mut exact_complete_request = |selected: &[usize]| -> Result<_, &'static str> {
            assert_eq!(selected, &[0]);
            Ok(RequestCapacityCount::new(
                stable + dynamic + history_with_c0 + tools,
                max_output_tokens,
            ))
        };
        let exact_budget = GroupAdmissionBudget {
            context_window: exact_window,
            safety_headroom: safety,
            group_upgrade_budget: 0,
            per_result_upgrade_ceiling: 0,
        };

        let plan =
            plan_group_admission(&results, exact_budget, &mut exact_complete_request).unwrap();
        assert_eq!(plan.final_capacity.input_upper_bound, 1_000);
        assert_eq!(plan.final_capacity.reserved_output, max_output_tokens);
        assert_eq!(plan.final_capacity.total_upper_bound(), 2_000);

        let mut one_token_too_small = exact_budget;
        one_token_too_small.context_window -= 1;
        let error =
            plan_group_admission(&results, one_token_too_small, &mut exact_complete_request)
                .unwrap_err();
        assert!(matches!(
            error,
            GroupAdmissionError::CurrentToolGroupEnvelopeOverflow { .. }
        ));
    }

    #[test]
    fn duplicate_call_ids_are_legal_when_result_keys_are_unique() {
        let results = ordered(vec![
            result("r1", "same-call", ResultAdmissionPriority::Unknown, &[1]),
            result("r2", "same-call", ResultAdmissionPriority::Unknown, &[1]),
        ]);

        let plan = plan_group_admission(&results, budget(0), &mut always_fits).unwrap();

        assert_eq!(plan.selections.len(), 2);
        assert!(plan
            .selections
            .iter()
            .all(|selection| selection.call_id == "same-call"));
    }

    #[test]
    fn duplicate_result_keys_are_rejected_even_when_call_ids_repeat() {
        let results = ordered(vec![
            result(
                "same-key",
                "same-call",
                ResultAdmissionPriority::Unknown,
                &[1],
            ),
            result(
                "same-key",
                "same-call",
                ResultAdmissionPriority::Unknown,
                &[1],
            ),
        ]);

        let error = plan_group_admission(&results, budget(0), &mut always_fits).unwrap_err();

        assert!(matches!(
            error,
            GroupAdmissionError::DuplicateResultKey { result_key }
                if result_key == "same-key"
        ));
    }

    #[test]
    fn empty_candidate_id_is_rejected() {
        let mut results = ordered(vec![result(
            "r1",
            "call-1",
            ResultAdmissionPriority::Unknown,
            &[1],
        )]);
        results[0].candidates[0].stable_id = "  ".to_string();

        let error = plan_group_admission(&results, budget(0), &mut always_fits).unwrap_err();

        assert!(matches!(
            error,
            GroupAdmissionError::EmptyCandidateId {
                result_index: 0,
                candidate_index: 0
            }
        ));
    }

    #[test]
    fn result_slice_must_be_sorted_by_contiguous_model_call_ordinal() {
        let mut results = ordered(vec![
            result("r1", "call-1", ResultAdmissionPriority::Unknown, &[1]),
            result("r2", "call-2", ResultAdmissionPriority::Unknown, &[1]),
        ]);
        let mut duplicate_ordinal = results.clone();
        duplicate_ordinal[1].model_call_ordinal = 0;
        let duplicate_error =
            plan_group_admission(&duplicate_ordinal, budget(0), &mut always_fits).unwrap_err();
        assert!(matches!(
            duplicate_error,
            GroupAdmissionError::InvalidModelCallOrder {
                result_index: 1,
                expected_ordinal: 1,
                actual_ordinal: 0
            }
        ));

        results.swap(0, 1);

        let error = plan_group_admission(&results, budget(0), &mut always_fits).unwrap_err();

        assert!(matches!(
            error,
            GroupAdmissionError::InvalidModelCallOrder {
                result_index: 0,
                expected_ordinal: 0,
                actual_ordinal: 1
            }
        ));
    }

    #[test]
    fn exact_c0_is_a_singleton_and_cannot_expand() {
        let exact = ResultCandidateSet::singleton_exact(
            "r1",
            "call-1",
            0,
            ResultAdmissionPriority::StructuredRead,
            "exact",
            32,
            CandidateTokenCount::new(4, 4, 4),
        );
        let plan =
            plan_group_admission(std::slice::from_ref(&exact), budget(100), &mut always_fits)
                .unwrap();
        assert!(plan.canonical_upgrades.is_empty());
        assert_eq!(plan.selections[0].candidate_stable_id, "exact");

        let mut invalid = exact;
        invalid.candidates.push(AdmissionCandidate {
            stable_id: "impossible-upgrade".to_string(),
            semantic_rank: 1,
            kind: AdmissionCandidateKind::OmissionPreview,
            source_bytes: 32,
            rendered_bytes: 16,
            tokens: CandidateTokenCount::new(5, 5, 5),
        });
        let error = plan_group_admission(&[invalid], budget(100), &mut always_fits).unwrap_err();
        assert!(matches!(
            error,
            GroupAdmissionError::InvalidExactCandidate {
                result_index: 0,
                candidate_index: 0
            }
        ));
    }

    #[test]
    fn exact_candidate_bytes_must_equal_source_bytes() {
        let mut exact = ResultCandidateSet::singleton_exact(
            "r1",
            "call-1",
            0,
            ResultAdmissionPriority::Unknown,
            "exact",
            32,
            CandidateTokenCount::new(4, 4, 4),
        );
        exact.candidates[0].rendered_bytes = 31;

        let error = plan_group_admission(&[exact], budget(0), &mut always_fits).unwrap_err();

        assert!(matches!(
            error,
            GroupAdmissionError::InvalidExactCandidate {
                result_index: 0,
                candidate_index: 0
            }
        ));
    }

    #[test]
    fn default_budget_is_only_c0_incremental_headroom() {
        let budget = GroupAdmissionBudget::tier1_defaults(32_000, 20_000).unwrap();

        assert_eq!(budget.safety_headroom, 512);
        assert_eq!(budget.group_upgrade_budget, 8_000);
        assert_eq!(budget.per_result_upgrade_ceiling, 3_200);
    }
}
