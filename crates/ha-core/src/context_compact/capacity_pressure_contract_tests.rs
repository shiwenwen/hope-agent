use super::{
    apply_capacity_pressure_tier, replay_capacity_pressure_edits, CapacityPressureTier,
    CompactConfig,
};
use serde_json::{json, Value};
use std::collections::HashSet;

fn exact_json_upper(history: &[Value]) -> anyhow::Result<u64> {
    Ok(serde_json::to_vec(history)?.len() as u64)
}

fn openai_chat_call_ids(history: &[Value]) -> HashSet<&str> {
    history
        .iter()
        .filter_map(|message| message.get("tool_calls").and_then(Value::as_array))
        .flatten()
        .filter_map(|call| call.get("id").and_then(Value::as_str))
        .collect()
}

fn openai_chat_result_ids(history: &[Value]) -> Vec<&str> {
    history
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .filter_map(|message| message.get("tool_call_id").and_then(Value::as_str))
        .collect()
}

#[test]
fn deterministic_tier0_then_tier2_strictly_reduce_one_exact_request_counter() {
    let mut history = vec![
        json!({"role":"user","content":"older request"}),
        json!({
            "role": "assistant",
            "tool_calls": [{"id":"old-eager","function":{"name":"grep"}}]
        }),
        json!({"role":"tool","tool_call_id":"old-eager","content":"e".repeat(12_000)}),
        json!({
            "role": "assistant",
            "tool_calls": [{"id":"old-read","function":{"name":"read"}}]
        }),
        json!({"role":"tool","tool_call_id":"old-read","content":"r".repeat(24_000)}),
        json!({"role":"user","content":"current request must stay verbatim"}),
        json!({
            "role": "assistant",
            "tool_calls": [
                {"id":"current-a","function":{"name":"read"}},
                {"id":"current-b","function":{"name":"grep"}}
            ]
        }),
        json!({"role":"tool","tool_call_id":"current-a","content":"CURRENT-A"}),
        json!({"role":"tool","tool_call_id":"current-b","content":"CURRENT-B"}),
    ];
    let protected_start = 5;
    let protected_suffix = history[protected_start..].to_vec();
    let config = CompactConfig::default();

    let tier0 = apply_capacity_pressure_tier(
        &mut history,
        protected_start,
        &config,
        CapacityPressureTier::Tier0,
        0,
        exact_json_upper,
    )
    .expect("Tier 0 pressure pass");
    assert_eq!(tier0.edits.len(), 1);
    assert!(tier0.input_upper_after < tier0.input_upper_before);
    assert!(!tier0.reached_target);
    assert_eq!(history[protected_start..], protected_suffix);

    let tier2 = apply_capacity_pressure_tier(
        &mut history,
        protected_start,
        &config,
        CapacityPressureTier::Tier2,
        0,
        exact_json_upper,
    )
    .expect("Tier 2 pressure pass");
    assert!(tier2.input_upper_after < tier2.input_upper_before);
    assert!(!tier2.reached_target);
    assert_eq!(history[protected_start..], protected_suffix);

    let calls = openai_chat_call_ids(&history);
    let results = openai_chat_result_ids(&history);
    assert_eq!(calls.len(), results.len());
    assert!(results.iter().all(|call_id| calls.contains(call_id)));
    assert_eq!(&results[results.len() - 2..], &["current-a", "current-b"]);
}

#[test]
fn pressure_edits_replay_by_ordinal_and_call_identity_without_touching_current_group() {
    let canonical = vec![
        json!({"role":"user","content":"older request"}),
        json!({"type":"function_call","call_id":"old","name":"grep","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"old","output":"__IMAGE_FILE__:canonical-marker"}),
        json!({"role":"user","content":"current request"}),
        json!({"type":"function_call","call_id":"current","name":"read","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"current","output":"CURRENT"}),
    ];
    let mut accounting = canonical.clone();
    accounting[2]["output"] = Value::String("vision transcription ".repeat(1_000));
    let protected_start = 3;
    let protected_accounting = accounting[protected_start..].to_vec();

    let pressure = apply_capacity_pressure_tier(
        &mut accounting,
        protected_start,
        &CompactConfig::default(),
        CapacityPressureTier::Tier0,
        0,
        exact_json_upper,
    )
    .expect("accounting pressure pass");
    assert_eq!(pressure.edits.len(), 1);
    assert_eq!(accounting[protected_start..], protected_accounting);

    let mut request_projection = canonical.clone();
    replay_capacity_pressure_edits(&mut request_projection, &pressure.edits)
        .expect("structurally equivalent replay");
    assert_eq!(
        request_projection[2]["output"],
        pressure.edits[0].replacement
    );
    assert_eq!(
        request_projection[protected_start..],
        canonical[protected_start..]
    );

    let mut identity_changed = canonical;
    identity_changed[2]["call_id"] = Value::String("different".to_string());
    assert!(replay_capacity_pressure_edits(&mut identity_changed, &pressure.edits).is_err());
}

#[test]
fn non_decreasing_counter_rolls_back_every_candidate_edit() {
    let mut history = vec![
        json!({"role":"user","content":"older request"}),
        json!({
            "role": "assistant",
            "tool_calls": [{"id":"old","function":{"name":"grep"}}]
        }),
        json!({"role":"tool","tool_call_id":"old","content":"x".repeat(12_000)}),
        json!({"role":"user","content":"current"}),
    ];
    let original = history.clone();
    let result = apply_capacity_pressure_tier(
        &mut history,
        3,
        &CompactConfig::default(),
        CapacityPressureTier::Tier0,
        0,
        |_history| Ok(10_000),
    )
    .expect("constant counter is valid but cannot prove an improvement");

    assert!(result.edits.is_empty());
    assert_eq!(result.input_upper_before, result.input_upper_after);
    assert!(!result.reached_target);
    assert_eq!(history, original);
}
