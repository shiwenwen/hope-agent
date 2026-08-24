//! Goal runner decisions and completion-report projection.

use serde_json::Value;

use ha_core::goal::{
    goal_has_current_satisfied_semantic_grade, goal_requires_semantic_grade, GoalClosureDecision,
    GoalCompletionReport, GoalSnapshot, GoalState,
};

pub fn runner_should_evaluate(snapshot: &GoalSnapshot) -> bool {
    matches!(
        snapshot.goal.state,
        GoalState::Active | GoalState::Evaluating | GoalState::Blocked
    ) && !snapshot.budget.exhausted
        && snapshot.goal.closure_decision != Some(GoalClosureDecision::AcceptedV1)
}

pub fn runner_should_continue(snapshot: &GoalSnapshot) -> bool {
    match snapshot.goal.state {
        GoalState::Active | GoalState::Evaluating => {}
        GoalState::Blocked => {
            let reason = snapshot.goal.blocked_reason.as_deref().unwrap_or_default();
            if !matches!(
                reason,
                "goal_evidence_incomplete" | "goal_blocked_by_evidence" | ""
            ) {
                return false;
            }
        }
        GoalState::Paused | GoalState::Completed | GoalState::Failed | GoalState::Cancelled => {
            return false;
        }
    }
    if snapshot.budget.exhausted {
        return false;
    }
    if snapshot.goal.closure_decision == Some(GoalClosureDecision::AcceptedV1) {
        return false;
    }
    let audit_status = snapshot
        .goal
        .final_evidence
        .get("status")
        .and_then(Value::as_str);
    let semantic_pending = goal_requires_semantic_grade(snapshot)
        && !goal_has_current_satisfied_semantic_grade(snapshot).unwrap_or(false);
    semantic_pending || audit_status != Some("completed") || snapshot.audit_stale
}

pub fn build_completion_report(
    snapshot: &GoalSnapshot,
    summary_override: Option<&str>,
) -> GoalCompletionReport {
    let audit = &snapshot.goal.final_evidence;
    let status = audit
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_else(|| snapshot.goal.state.as_str())
        .to_string();
    let summary = summary_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| snapshot.goal.final_summary.clone())
        .or_else(|| {
            audit
                .get("summary")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            if snapshot.goal.state == GoalState::Completed {
                "Goal completed.".to_string()
            } else {
                "Goal is not complete yet.".to_string()
            }
        });

    let mut follow_up_items = string_vec(audit.get("followUpItems"));
    for item in &snapshot.goal.follow_up_items {
        if !follow_up_items
            .iter()
            .any(|existing| existing == &item.text)
        {
            follow_up_items.push(item.text.clone());
        }
    }

    GoalCompletionReport {
        goal_id: snapshot.goal.id.clone(),
        session_id: snapshot.goal.session_id.clone(),
        state: snapshot.goal.state,
        status,
        objective: snapshot.goal.objective.clone(),
        revision: snapshot.goal.revision,
        summary,
        usage: snapshot.budget.clone(),
        evidence_count: snapshot.evidence.len(),
        achieved: string_vec(audit.get("achieved")),
        missing: string_vec(audit.get("missing")),
        blockers: string_vec(audit.get("blockers")),
        follow_up_items,
        remaining_risk: audit
            .get("remainingRisk")
            .and_then(Value::as_str)
            .map(str::to_string),
        generated_at: ha_core::util::now_rfc3339(),
    }
}

fn string_vec(value: Option<&Value>) -> Vec<String> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(text) = item.as_str() {
                return Some(text.to_string());
            }
            item.get("text")
                .or_else(|| item.get("summary"))
                .or_else(|| item.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|item| !item.trim().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::string_vec;
    use serde_json::json;

    #[test]
    fn report_lists_preserve_legacy_object_entries() {
        assert_eq!(
            string_vec(Some(&json!([
                "plain",
                { "text": "text item" },
                { "summary": "summary item" },
                { "reason": "reason item" },
                { "ignored": true }
            ]))),
            vec!["plain", "text item", "summary item", "reason item"]
        );
    }
}
