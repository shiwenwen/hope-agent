// ── API-Round Message Grouping ──
//
// Stamps `_oc_round` metadata on messages during tool loops so that
// compaction (Tier 3 summarization, Tier 4 emergency) never splits
// a tool_use/tool_result pair. The metadata is stripped before API calls.
//
// Round ID format: "r{N}" where N is the tool-loop iteration index.
// Messages without `_oc_round` (from older sessions) are treated as
// individual rounds — backward compatible with existing behavior.

use serde_json::Value;

/// Metadata key for round ID, stamped in tool loops, stripped before API calls.
pub const ROUND_KEY: &str = "_oc_round";

/// Stamp a round ID on a message (in-place).
pub fn stamp_round(msg: &mut Value, round_id: &str) {
    if let Some(obj) = msg.as_object_mut() {
        obj.insert(ROUND_KEY.to_string(), Value::String(round_id.to_string()));
    }
}

/// Push a message and stamp it with the current tool-loop round index.
/// Convenience helper that avoids repeating `push + stamp_round(last_mut().unwrap())`
/// across all 4 provider files.
pub fn push_and_stamp(messages: &mut Vec<Value>, mut msg: Value, round: u32) {
    stamp_round(&mut msg, &format!("r{}", round));
    messages.push(msg);
}

/// Strip round metadata from a single message (in-place, idempotent).
pub fn strip_round(msg: &mut Value) {
    if let Some(obj) = msg.as_object_mut() {
        obj.remove(ROUND_KEY);
    }
}

/// Strip round metadata from all messages (in-place).
#[allow(dead_code)]
pub fn strip_rounds(messages: &mut [Value]) {
    for msg in messages.iter_mut() {
        strip_round(msg);
    }
}

/// Get the round ID of a message, if stamped.
fn get_round(msg: &Value) -> Option<&str> {
    msg.get(ROUND_KEY).and_then(|v| v.as_str())
}

/// Clone messages and strip round metadata (for API request body construction).
/// This avoids modifying the working message vec while producing a clean copy for the API.
pub fn prepare_messages_for_api(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            let mut clean = m.clone();
            strip_round(&mut clean);
            clean
        })
        .collect()
}

/// Find the nearest round-safe split point at or before `target_index`.
///
/// A round-safe point is one where the message at `index` does NOT share
/// a round with the message at `index - 1`. This guarantees that cutting
/// at this index keeps all messages within a round on the same side.
///
/// Falls back to `target_index` if no `_oc_round` metadata exists
/// (backward compatibility with older sessions).
pub fn find_round_safe_boundary(messages: &[Value], target_index: usize) -> usize {
    if target_index == 0 || target_index >= messages.len() {
        return target_index;
    }

    // Walk backward from target_index to find a boundary where
    // adjacent messages have different round IDs (or no round metadata).
    for i in (1..=target_index).rev() {
        let curr_round = get_round(&messages[i]);
        let prev_round = get_round(&messages[i - 1]);

        match (prev_round, curr_round) {
            (Some(prev), Some(curr)) if prev == curr => continue, // same round, keep looking
            _ => return i, // different rounds or no metadata = safe boundary
        }
    }

    // All messages share the same round (shouldn't happen normally) — return 0
    // to avoid splitting inside the single round.
    0
}

/// Find the nearest round-safe split point at or after `target_index`.
///
/// Used by `emergency_compact()` where we want to keep messages from
/// `target_index` onward. If `target_index` falls mid-round, walks forward
/// to find the start of the next round.
///
/// Falls back to `target_index` if no `_oc_round` metadata exists.
pub fn find_round_safe_boundary_forward(messages: &[Value], target_index: usize) -> usize {
    if target_index == 0 || target_index >= messages.len() {
        return target_index;
    }

    // Check if target_index is already at a round boundary
    let target_round = get_round(&messages[target_index]);
    let prev_round = get_round(&messages[target_index - 1]);

    match (prev_round, target_round) {
        (Some(prev), Some(curr)) if prev == curr => {
            // target is mid-round, walk forward to find the start of the next round
            for i in (target_index + 1)..messages.len() {
                let curr = get_round(&messages[i]);
                let prev = get_round(&messages[i - 1]);
                match (prev, curr) {
                    (Some(p), Some(c)) if p == c => continue,
                    _ => return i,
                }
            }
            // All remaining messages are one round — keep them all
            target_index
        }
        _ => target_index, // already at boundary or no metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(role: &str, round: Option<&str>) -> Value {
        let mut m = json!({ "role": role, "content": "test" });
        if let Some(r) = round {
            stamp_round(&mut m, r);
        }
        m
    }

    #[test]
    fn test_stamp_and_strip() {
        let mut m = json!({ "role": "assistant" });
        stamp_round(&mut m, "r0");
        assert_eq!(get_round(&m), Some("r0"));
        strip_round(&mut m);
        assert_eq!(get_round(&m), None);
    }

    #[test]
    fn test_prepare_messages_for_api() {
        let mut messages = vec![
            json!({ "role": "user", "content": "hi" }),
            json!({ "role": "assistant", "content": "hello" }),
        ];
        stamp_round(&mut messages[1], "r0");
        let api = prepare_messages_for_api(&messages);
        assert!(api[1].get(ROUND_KEY).is_none());
        // Original still has the stamp
        assert!(messages[1].get(ROUND_KEY).is_some());
    }

    #[test]
    fn test_find_round_safe_boundary_no_metadata() {
        // Without metadata, target_index is returned as-is (backward compat)
        let messages = vec![msg("user", None), msg("assistant", None), msg("user", None)];
        assert_eq!(find_round_safe_boundary(&messages, 1), 1);
        assert_eq!(find_round_safe_boundary(&messages, 2), 2);
    }

    #[test]
    fn test_find_round_safe_boundary_mid_round() {
        // [user] [assistant r0] [tool r0] [user] [assistant r1] [tool r1]
        let messages = vec![
            msg("user", None),
            msg("assistant", Some("r0")),
            msg("tool", Some("r0")),
            msg("user", None),
            msg("assistant", Some("r1")),
            msg("tool", Some("r1")),
        ];
        // target=2 is mid-round r0 → should snap back to 1 (start of r0)
        assert_eq!(find_round_safe_boundary(&messages, 2), 1);
        // target=3 is between rounds → safe at 3
        assert_eq!(find_round_safe_boundary(&messages, 3), 3);
        // target=5 is mid-round r1 → should snap back to 4
        assert_eq!(find_round_safe_boundary(&messages, 5), 4);
    }

    #[test]
    fn test_find_round_safe_boundary_forward_mid_round() {
        let messages = vec![
            msg("user", None),
            msg("assistant", Some("r0")),
            msg("tool", Some("r0")),
            msg("tool", Some("r0")),
            msg("user", None),
            msg("assistant", Some("r1")),
        ];
        // target=2 is mid-round r0 → walk forward to 4 (first non-r0)
        assert_eq!(find_round_safe_boundary_forward(&messages, 2), 4);
        // target=4 is between rounds → safe at 4
        assert_eq!(find_round_safe_boundary_forward(&messages, 4), 4);
    }

    #[test]
    fn test_edge_cases() {
        assert_eq!(find_round_safe_boundary(&[], 0), 0);
        assert_eq!(find_round_safe_boundary_forward(&[], 0), 0);

        let single = vec![msg("user", None)];
        assert_eq!(find_round_safe_boundary(&single, 0), 0);
        assert_eq!(find_round_safe_boundary_forward(&single, 0), 0);
    }
}
