#[cfg(test)]
mod tests {
    use crate::plan::*;

    #[test]
    fn test_plan_mode_state_roundtrip() {
        assert_eq!(PlanModeState::from_str("planning"), PlanModeState::Planning);
        assert_eq!(PlanModeState::from_str("review"), PlanModeState::Review);
        assert_eq!(
            PlanModeState::from_str("executing"),
            PlanModeState::Executing
        );
        assert_eq!(
            PlanModeState::from_str("completed"),
            PlanModeState::Completed
        );
        assert_eq!(PlanModeState::from_str("off"), PlanModeState::Off);
        assert_eq!(PlanModeState::from_str("unknown"), PlanModeState::Off);
        // "paused" is no longer a valid state — falls back to Off.
        assert_eq!(PlanModeState::from_str("paused"), PlanModeState::Off);
        assert_eq!(PlanModeState::Planning.as_str(), "planning");
        assert_eq!(PlanModeState::Review.as_str(), "review");
        assert_eq!(PlanModeState::Completed.as_str(), "completed");
    }

    #[test]
    fn test_plan_mode_transitions() {
        // Forward flow
        assert!(PlanModeState::Planning.is_valid_transition(&PlanModeState::Review));
        assert!(PlanModeState::Review.is_valid_transition(&PlanModeState::Executing));
        assert!(PlanModeState::Executing.is_valid_transition(&PlanModeState::Completed));

        // Re-entry: Executing/Completed → Planning to revise the approved plan
        assert!(PlanModeState::Executing.is_valid_transition(&PlanModeState::Planning));
        assert!(PlanModeState::Completed.is_valid_transition(&PlanModeState::Planning));

        // Off escape hatch is always allowed
        assert!(PlanModeState::Executing.is_valid_transition(&PlanModeState::Off));
        assert!(PlanModeState::Off.is_valid_transition(&PlanModeState::Planning));

        // Illegal: skipping the review checkpoint, or jumping past Executing
        assert!(!PlanModeState::Planning.is_valid_transition(&PlanModeState::Executing));
        assert!(!PlanModeState::Review.is_valid_transition(&PlanModeState::Completed));
        assert!(!PlanModeState::Completed.is_valid_transition(&PlanModeState::Executing));
    }

    /// `transition_state` is the single helper every caller must funnel
    /// through (F-037). Unit-testing it without globals proves the in-memory
    /// contract: legal edges → Applied; illegal edges → Rejected without
    /// touching the existing meta. (DB persist + event emit are exercised
    /// only when globals are registered, which is the integration domain.)
    #[tokio::test]
    async fn test_transition_state_in_memory_contract() {
        // Each test case uses a unique session id so parallel test runs
        // don't fight over the global plan store.
        let sid_apply = "transition_test_apply";
        let sid_reject = "transition_test_reject";

        // Start clean.
        set_plan_state(sid_apply, PlanModeState::Off).await;
        set_plan_state(sid_reject, PlanModeState::Off).await;

        // Off → Planning is valid.
        let outcome = transition_state(
            sid_apply,
            PlanModeState::Planning,
            TransitionOpts::new("test_apply"),
        )
        .await
        .expect("transition should not error without DB / event bus");
        assert_eq!(outcome, TransitionOutcome::Applied);
        assert_eq!(get_plan_state(sid_apply).await, PlanModeState::Planning);

        // Off → Planning, then Planning → Completed (illegal: must go via Review).
        transition_state(
            sid_reject,
            PlanModeState::Planning,
            TransitionOpts::new("test_setup"),
        )
        .await
        .expect("setup transition");
        let outcome = transition_state(
            sid_reject,
            PlanModeState::Completed,
            TransitionOpts::new("test_reject"),
        )
        .await
        .expect("rejection still returns Ok, just with Rejected variant");
        assert_eq!(outcome, TransitionOutcome::Rejected);
        // Rejected transitions must leave the in-memory state intact.
        assert_eq!(get_plan_state(sid_reject).await, PlanModeState::Planning);

        // Cleanup.
        set_plan_state(sid_apply, PlanModeState::Off).await;
        set_plan_state(sid_reject, PlanModeState::Off).await;
    }
}
