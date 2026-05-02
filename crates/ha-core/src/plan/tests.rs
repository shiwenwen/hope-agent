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

    /// Legal edges return Applied; illegal edges return Rejected without
    /// touching the existing meta. DB persist + event emit are skipped when
    /// globals are unregistered (test environment), exercising only the
    /// in-memory contract.
    #[tokio::test]
    async fn test_transition_state_in_memory_contract() {
        // Unique session ids so parallel test runs don't fight over the
        // global plan store.
        let sid_apply = "transition_test_apply";
        let sid_reject = "transition_test_reject";

        set_plan_state(sid_apply, PlanModeState::Off).await;
        set_plan_state(sid_reject, PlanModeState::Off).await;

        let outcome = transition_state(sid_apply, PlanModeState::Planning, "test_apply")
            .await
            .expect("transition should not error without DB / event bus");
        assert_eq!(outcome, TransitionOutcome::Applied);
        assert_eq!(get_plan_state(sid_apply).await, PlanModeState::Planning);

        // Planning → Completed is illegal (must go via Review).
        transition_state(sid_reject, PlanModeState::Planning, "test_setup")
            .await
            .expect("setup transition");
        let outcome = transition_state(sid_reject, PlanModeState::Completed, "test_reject")
            .await
            .expect("rejection still returns Ok, just with Rejected variant");
        assert_eq!(outcome, TransitionOutcome::Rejected);
        assert_eq!(get_plan_state(sid_reject).await, PlanModeState::Planning);

        set_plan_state(sid_apply, PlanModeState::Off).await;
        set_plan_state(sid_reject, PlanModeState::Off).await;
    }

    /// Transitions INTO Executing must stamp `executing_started_at`. The
    /// stamp is the boundary that scopes `maybe_complete_plan` to plan-period
    /// tasks, so leftover pending tasks from before approval don't block
    /// auto-completion (and stale completed tasks don't trigger it).
    #[tokio::test]
    async fn test_executing_started_at_is_stamped() {
        let sid = "transition_test_executing_stamp";
        set_plan_state(sid, PlanModeState::Off).await;

        // Walk through Planning → Review → Executing so the state machine
        // accepts the edges. Only the Executing transition should stamp.
        transition_state(sid, PlanModeState::Planning, "test")
            .await
            .expect("ok");
        assert!(
            get_plan_meta(sid)
                .await
                .and_then(|m| m.executing_started_at)
                .is_none(),
            "Planning should not stamp executing_started_at"
        );

        transition_state(sid, PlanModeState::Review, "test")
            .await
            .expect("ok");
        assert!(
            get_plan_meta(sid)
                .await
                .and_then(|m| m.executing_started_at)
                .is_none(),
            "Review should not stamp executing_started_at"
        );

        transition_state(sid, PlanModeState::Executing, "test")
            .await
            .expect("ok");
        let stamp = get_plan_meta(sid)
            .await
            .and_then(|m| m.executing_started_at)
            .expect("Executing transition must stamp executing_started_at");
        assert!(
            chrono::DateTime::parse_from_rfc3339(&stamp).is_ok(),
            "stamp should be valid rfc3339, got {}",
            stamp
        );

        set_plan_state(sid, PlanModeState::Off).await;
    }

    /// Transitions to Completed via `transition_state` must clear the in-meta
    /// `checkpoint_ref` so `get_plan_checkpoint` doesn't return a deleted
    /// branch (the front-end Rollback button check). Off transitions remove
    /// the entire PlanMeta, so they don't need this; Completed keeps the meta
    /// and must explicitly null the field.
    #[tokio::test]
    async fn test_completed_clears_stale_checkpoint_ref() {
        let sid = "transition_test_stale_ref";
        set_plan_state(sid, PlanModeState::Off).await;

        // Bring the session into Executing and inject a fake checkpoint_ref
        // directly into the store (skipping git.rs::create_checkpoint_for_session
        // which would try to actually run git in the test).
        transition_state(sid, PlanModeState::Planning, "test")
            .await
            .expect("ok");
        transition_state(sid, PlanModeState::Review, "test")
            .await
            .expect("ok");
        transition_state(sid, PlanModeState::Executing, "test")
            .await
            .expect("ok");
        {
            let mut map = store().write().await;
            if let Some(meta) = map.get_mut(sid) {
                meta.checkpoint_ref = Some("hope-plan-checkpoint-fake".to_string());
            }
        }
        assert_eq!(
            get_checkpoint_ref(sid).await.as_deref(),
            Some("hope-plan-checkpoint-fake"),
            "precondition: stale ref present"
        );

        // Transition to Completed and confirm the ref is cleared.
        transition_state(sid, PlanModeState::Completed, "test")
            .await
            .expect("ok");
        assert!(
            get_checkpoint_ref(sid).await.is_none(),
            "Completed must clear stale checkpoint_ref"
        );

        set_plan_state(sid, PlanModeState::Off).await;
    }
}
