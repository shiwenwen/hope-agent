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
}
