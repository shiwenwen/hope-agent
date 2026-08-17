// ── Unified Recent Boundary ─────────────────────────────────────
//
// Computes the single protected recent-region boundary used by Tier 0/2/3/4.
// The boundary is round-safe and expands to the user turn that owns the
// protected assistant/tool rounds, so the latest user request remains verbatim.

use serde_json::Value;
use std::collections::HashMap;

use super::estimation::{
    first_tool_result_id, is_tool_call, is_tool_result, is_user_message, message_role,
    message_type, tool_call_ids, tool_result_ids,
};
use super::round_grouping::ROUND_KEY;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRound {
    pub round_id: Option<String>,
    pub user_turn_start: usize,
    pub start: usize,
    pub end_exclusive: usize,
    pub kind: RoundKind,
    pub has_tool_call: bool,
    pub has_tool_result: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundKind {
    UserOnly,
    AssistantText,
    ToolRound,
    Recovered,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentBoundary {
    pub protected_start_index: usize,
    pub rounds: Vec<MessageRound>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryMode {
    /// Tier 0/2 normal compaction: fail closed if there is no clear prunable
    /// prefix.
    ProtectRecent,
    /// Tier 3 is already above the summary threshold. It should still preserve
    /// the latest live round, but it may summarize earlier rounds even when the
    /// normal recent boundary would fail closed.
    SummarizeUnderPressure,
    /// Tier 4 is the final ContextOverflow fallback and must shrink if possible.
    Emergency,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundarySnapshot {
    pub rounds: Vec<MessageRound>,
    live_round_indices: Vec<usize>,
    preserve_recent_rounds: usize,
}

/// Exact provider-native user item that anchors the active request across a
/// destructive Tier-4 projection.
///
/// The value is captured before compaction and checked after compaction. This
/// intentionally compares the whole provider item instead of display text: two
/// different requests may have the same text while attachments or provider
/// metadata differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LatestUserRequestAnchor {
    message: Value,
}

impl LatestUserRequestAnchor {
    /// Tier 4 may delete older duplicate-looking turns, but the exact latest
    /// user item must remain once and only once in the projected history.
    pub(crate) fn is_preserved_exactly_once(&self, messages: &[Value]) -> bool {
        messages
            .iter()
            .filter(|message| is_user_message(message) && *message == &self.message)
            .count()
            == 1
    }
}

pub(crate) fn latest_user_request_anchor(messages: &[Value]) -> Option<LatestUserRequestAnchor> {
    messages
        .iter()
        .rev()
        .find(|message| is_user_message(message))
        .cloned()
        .map(|message| LatestUserRequestAnchor { message })
}

/// Return the genuine user-turn start that owns `message_index`.
///
/// Anthropic tool-result containers also use `role=user`, so callers must not
/// recover this boundary with a raw reverse role scan. `build_message_rounds`
/// already distinguishes genuine user input from result containers and keeps
/// the owning turn on every protocol-safe round.
pub(crate) fn user_turn_start_for_message(
    messages: &[Value],
    message_index: usize,
) -> Option<usize> {
    build_message_rounds(messages)
        .into_iter()
        .find(|round| round.start <= message_index && message_index < round.end_exclusive)
        .map(|round| round.user_turn_start)
}

fn round_id(msg: &Value) -> Option<&str> {
    msg.get(ROUND_KEY).and_then(|v| v.as_str())
}

fn is_recovered_round_id(id: Option<&str>) -> bool {
    id.is_some_and(super::round_grouping::is_recovered_round)
}

fn classify_round(
    messages: &[Value],
    start: usize,
    end_exclusive: usize,
) -> (RoundKind, bool, bool) {
    let slice = &messages[start..end_exclusive];
    let mut has_tool_call = false;
    let mut has_tool_result = false;
    let mut has_assistant = false;
    let mut all_user = true;
    for msg in slice {
        has_tool_call |= is_tool_call(msg);
        has_tool_result |= is_tool_result(msg);
        has_assistant |= message_role(msg) == Some("assistant")
            || (message_type(msg) == Some("message") && message_role(msg) != Some("user"));
        all_user &= is_user_message(msg);
    }
    if has_tool_call || has_tool_result {
        return (RoundKind::ToolRound, has_tool_call, has_tool_result);
    }
    if has_assistant {
        return (RoundKind::AssistantText, has_tool_call, has_tool_result);
    }
    if all_user {
        return (RoundKind::UserOnly, has_tool_call, has_tool_result);
    }
    (RoundKind::Unknown, has_tool_call, has_tool_result)
}

fn build_tool_result_index(messages: &[Value]) -> HashMap<String, usize> {
    let mut result_index = HashMap::new();
    for (idx, msg) in messages.iter().enumerate() {
        for id in tool_result_ids(msg) {
            result_index.entry(id.to_string()).or_insert(idx);
        }
    }
    result_index
}

/// Build best-effort message rounds across stamped and unstamped histories.
pub fn build_message_rounds(messages: &[Value]) -> Vec<MessageRound> {
    let mut rounds = Vec::new();
    let mut i = 0;
    let mut current_user_turn_start = 0;
    let tool_result_index = build_tool_result_index(messages);
    let mut call_id_to_round_index: HashMap<String, usize> = HashMap::new();

    while i < messages.len() {
        if is_user_message(&messages[i]) {
            current_user_turn_start = i;
        }

        let id = round_id(&messages[i]).map(str::to_string);
        if let Some(id_ref) = round_id(&messages[i]) {
            let start = i;
            i += 1;
            while i < messages.len() && round_id(&messages[i]) == Some(id_ref) {
                i += 1;
            }
            let (mut kind, has_tool_call, has_tool_result) = classify_round(messages, start, i);
            if is_recovered_round_id(Some(id_ref)) {
                kind = RoundKind::Recovered;
            }
            rounds.push(MessageRound {
                round_id: id,
                user_turn_start: current_user_turn_start,
                start,
                end_exclusive: i,
                has_tool_call,
                has_tool_result,
                kind,
            });
            continue;
        }

        let start = i;
        let mut end = i + 1;
        if is_tool_call(&messages[i]) {
            for call_id in tool_call_ids(&messages[i]) {
                if let Some(result_idx) = tool_result_index.get(call_id).copied() {
                    end = end.max(result_idx + 1);
                }
            }
        } else if is_tool_result(&messages[i]) {
            let mut merged_with_previous = false;
            if let Some(result_id) = first_tool_result_id(&messages[i]) {
                if let Some(round_idx) = call_id_to_round_index.get(result_id).copied() {
                    if let Some(prev) = rounds.get_mut(round_idx) {
                        if prev.end_exclusive == i {
                            prev.end_exclusive = i + 1;
                            prev.has_tool_result = true;
                            prev.kind = RoundKind::ToolRound;
                            i += 1;
                            merged_with_previous = true;
                        }
                    }
                }
            }
            if merged_with_previous {
                continue;
            }
        }

        let (kind, has_tool_call, has_tool_result) = classify_round(messages, start, end);
        let round_idx = rounds.len();
        let round = MessageRound {
            round_id: None,
            user_turn_start: current_user_turn_start,
            start,
            end_exclusive: end,
            has_tool_call,
            has_tool_result,
            kind,
        };
        if round.has_tool_call {
            for msg in &messages[start..end] {
                for call_id in tool_call_ids(msg) {
                    call_id_to_round_index.insert(call_id.to_string(), round_idx);
                }
            }
        }
        rounds.push(round);
        i = end;
    }

    rounds
}

impl BoundarySnapshot {
    pub fn new(messages: &[Value], preserve_recent_rounds: usize) -> Self {
        let rounds = build_message_rounds(messages);
        let live_round_indices = rounds
            .iter()
            .enumerate()
            .filter_map(|(idx, round)| (round.kind != RoundKind::Recovered).then_some(idx))
            .collect();
        Self {
            rounds,
            live_round_indices,
            preserve_recent_rounds: preserve_recent_rounds.max(1),
        }
    }

    fn latest_live_round_start(&self) -> Option<usize> {
        self.live_round_indices
            .last()
            .and_then(|idx| self.rounds.get(*idx))
            .map(|round| round.start)
    }

    fn latest_user_turn_start(&self) -> Option<usize> {
        self.live_round_indices
            .last()
            .and_then(|idx| self.rounds.get(*idx))
            .map(|round| round.user_turn_start)
    }

    pub fn boundary(&self, messages: &[Value], mode: BoundaryMode) -> RecentBoundary {
        let mut warnings = Vec::new();
        if messages.is_empty() || self.rounds.is_empty() {
            return RecentBoundary {
                protected_start_index: messages.len(),
                rounds: self.rounds.clone(),
                warnings,
            };
        }

        let relax_to_latest_round = |warnings: &mut Vec<String>, reason: &str| {
            warnings.push(reason.to_string());
            self.latest_live_round_start().unwrap_or(0)
        };

        let mut protected_start_index = if self.live_round_indices.len()
            <= self.preserve_recent_rounds
        {
            warnings.push("not_enough_rounds_for_prunable_prefix".to_string());
            match mode {
                BoundaryMode::ProtectRecent => 0,
                BoundaryMode::SummarizeUnderPressure => {
                    relax_to_latest_round(&mut warnings, "summary_boundary_relaxed_to_latest_round")
                }
                BoundaryMode::Emergency => {
                    relax_to_latest_round(&mut warnings, "emergency_boundary_kept_latest_round")
                }
            }
        } else {
            let first_protected_round = self.live_round_indices
                [self.live_round_indices.len() - self.preserve_recent_rounds];
            let first_round = &self.rounds[first_protected_round];
            let user_turn_has_prior_execution_rounds =
                self.rounds.iter().take(first_protected_round).any(|round| {
                    round.user_turn_start == first_round.user_turn_start
                        && matches!(round.kind, RoundKind::AssistantText | RoundKind::ToolRound)
                });

            let candidate = if user_turn_has_prior_execution_rounds {
                warnings.push("user_turn_expansion_limited_by_prior_execution_rounds".to_string());
                first_round.start
            } else {
                first_round.user_turn_start
            };

            if candidate == 0 {
                match mode {
                    BoundaryMode::ProtectRecent => candidate,
                    BoundaryMode::SummarizeUnderPressure => relax_to_latest_round(
                        &mut warnings,
                        "summary_boundary_relaxed_to_latest_round",
                    ),
                    BoundaryMode::Emergency => {
                        relax_to_latest_round(&mut warnings, "emergency_boundary_kept_latest_round")
                    }
                }
            } else {
                candidate
            }
        };

        protected_start_index =
            super::round_grouping::find_round_safe_boundary(messages, protected_start_index);

        // Tier 3 and Tier 4 both replace active history, so the current user
        // request is a hard handoff anchor. Under pressure we may protect fewer
        // assistant/tool rounds than the normal policy, but must never move the
        // cut past the latest genuine user message. If that leaves no old
        // prefix, the caller must use content reduction or fail closed rather
        // than retry a request that has forgotten its task.
        if matches!(
            mode,
            BoundaryMode::SummarizeUnderPressure | BoundaryMode::Emergency
        ) {
            if let Some(latest_user_start) = self.latest_user_turn_start() {
                if protected_start_index > latest_user_start {
                    protected_start_index = super::round_grouping::find_round_safe_boundary(
                        messages,
                        latest_user_start,
                    );
                    warnings.push(
                        match mode {
                            BoundaryMode::SummarizeUnderPressure => {
                                "summary_boundary_preserved_latest_user_request"
                            }
                            BoundaryMode::Emergency => {
                                "emergency_boundary_preserved_latest_user_request"
                            }
                            BoundaryMode::ProtectRecent => {
                                unreachable!("only replacement modes enter the latest-user clamp")
                            }
                        }
                        .to_string(),
                    );
                }
            }
        }

        RecentBoundary {
            protected_start_index,
            rounds: self.rounds.clone(),
            warnings,
        }
    }
}

pub fn boundary_snapshot(messages: &[Value], preserve_recent_rounds: usize) -> BoundarySnapshot {
    BoundarySnapshot::new(messages, preserve_recent_rounds)
}

/// Compute the protected recent boundary. If there are not enough rounds to
/// leave any summarizable/prunable prefix, fail closed and protect everything.
pub fn recent_boundary(messages: &[Value], preserve_recent_rounds: usize) -> RecentBoundary {
    boundary_snapshot(messages, preserve_recent_rounds)
        .boundary(messages, BoundaryMode::ProtectRecent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn responses_tool_round_stays_with_user_turn() {
        let messages = vec![
            json!({"role":"user","content":"old"}),
            json!({"role":"assistant","content":"old reply"}),
            json!({"role":"user","content":"latest request"}),
            json!({"type":"function_call","call_id":"fc_1","name":"web_search","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"fc_1","output":"result"}),
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}),
        ];

        let boundary = recent_boundary(&messages, 2);

        assert_eq!(boundary.protected_start_index, 2);
    }

    #[test]
    fn emergency_boundary_never_moves_past_current_user_before_tool_round() {
        let messages = vec![
            json!({"role":"user","content":"old request"}),
            json!({"role":"assistant","content":"old reply"}),
            json!({"role":"user","content":"current request"}),
            json!({
                "role":"assistant",
                "content":null,
                "tool_calls":[{
                    "id":"call_1",
                    "type":"function",
                    "function":{"name":"read_file","arguments":"{}"}
                }],
                "_oc_round":"r0"
            }),
            json!({
                "role":"tool",
                "tool_call_id":"call_1",
                "content":"result",
                "_oc_round":"r0"
            }),
        ];

        let boundary = boundary_snapshot(&messages, 8).boundary(&messages, BoundaryMode::Emergency);

        assert_eq!(boundary.protected_start_index, 2);
        assert!(boundary
            .warnings
            .iter()
            .any(|warning| warning == "emergency_boundary_preserved_latest_user_request"));
    }

    #[test]
    fn responses_parallel_tool_calls_remain_one_tool_round_until_all_outputs() {
        let messages = vec![
            json!({"role":"user","content":"run two searches"}),
            json!({"type":"function_call","call_id":"fc_1","name":"grep","arguments":"{}"}),
            json!({"type":"function_call","call_id":"fc_2","name":"find","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"fc_1","output":"grep result"}),
            json!({"type":"function_call_output","call_id":"fc_2","output":"find result"}),
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}),
        ];

        let rounds = build_message_rounds(&messages);

        assert_eq!(rounds.len(), 3);
        assert_eq!(rounds[1].start, 1);
        assert_eq!(rounds[1].end_exclusive, 5);
        assert!(rounds[1].has_tool_call);
        assert!(rounds[1].has_tool_result);
    }

    #[test]
    fn openai_chat_tool_round_stays_with_user_turn() {
        let messages = vec![
            json!({"role":"user","content":"old"}),
            json!({"role":"assistant","content":"old reply"}),
            json!({"role":"user","content":"latest request"}),
            json!({
                "role":"assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "read_file", "arguments": "{}"}
                }]
            }),
            json!({"role":"tool","tool_call_id":"call_1","content":"result"}),
            json!({"role":"assistant","content":"done"}),
        ];

        let rounds = build_message_rounds(&messages);
        assert_eq!(rounds[2].start, 2);
        assert_eq!(rounds[3].start, 3);
        assert_eq!(rounds[3].end_exclusive, 5);

        let boundary = recent_boundary(&messages, 2);
        assert_eq!(boundary.protected_start_index, 2);
    }

    #[test]
    fn anthropic_tool_round_stays_with_result_block() {
        let messages = vec![
            json!({"role":"user","content":"old"}),
            json!({"role":"assistant","content":"old reply"}),
            json!({"role":"user","content":"latest request"}),
            json!({
                "role":"assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "read_file",
                    "input": {"path": "src/lib.rs"}
                }]
            }),
            json!({
                "role":"user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_1",
                    "content": "result"
                }]
            }),
            json!({"role":"assistant","content":"done"}),
        ];

        let rounds = build_message_rounds(&messages);
        assert_eq!(rounds[3].start, 3);
        assert_eq!(rounds[3].end_exclusive, 5);

        let boundary = recent_boundary(&messages, 2);
        assert_eq!(boundary.protected_start_index, 2);
    }

    #[test]
    fn recovered_rounds_do_not_count_toward_recent_preserve() {
        let recovered = super::super::round_grouping::recovered_round_id();
        let mut recovered_msg = json!({"role":"user","content":"snapshot"});
        recovered_msg
            .as_object_mut()
            .unwrap()
            .insert(ROUND_KEY.to_string(), json!(recovered));
        let messages = vec![
            json!({"role":"user","content":"old"}),
            json!({"role":"assistant","content":"old reply"}),
            json!({"role":"user","content":"newer"}),
            json!({"role":"assistant","content":"newer reply"}),
            recovered_msg,
        ];

        let boundary = recent_boundary(&messages, 1);
        assert_eq!(boundary.protected_start_index, 2);
    }

    #[test]
    fn long_tool_loop_limits_user_turn_expansion() {
        let messages = vec![
            json!({"role":"user","content":"inspect a large repo"}),
            json!({"type":"function_call","call_id":"fc_1","name":"ls","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"fc_1","output":"ls result"}),
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"next"}]}),
            json!({"type":"function_call","call_id":"fc_2","name":"grep","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"fc_2","output":"grep result"}),
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"next"}]}),
            json!({"type":"function_call","call_id":"fc_3","name":"find","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"fc_3","output":"find result"}),
        ];

        let boundary = recent_boundary(&messages, 2);

        assert!(boundary.protected_start_index > 0);
        assert_eq!(boundary.protected_start_index, 6);
        assert!(boundary
            .warnings
            .contains(&"user_turn_expansion_limited_by_prior_execution_rounds".to_string()));
    }

    #[test]
    fn fail_closed_when_not_enough_rounds() {
        let messages = vec![
            json!({"role":"user","content":"one"}),
            json!({"role":"assistant","content":"two"}),
        ];

        let boundary = recent_boundary(&messages, 4);

        assert_eq!(boundary.protected_start_index, 0);
        assert_eq!(
            boundary.warnings,
            vec!["not_enough_rounds_for_prunable_prefix"]
        );
    }
}
