#[cfg(test)]
mod tests {
    use crate::plan::*;

    #[test]
    fn test_parse_plan_steps() {
        let md = "\
### Phase 1: Analysis
- [ ] Read config files at src/config.ts
- [x] Analyze CSS variables in theme.css
### Phase 2: Implementation
- [ ] Add ThemeProvider component
- [ ] Create toggle button";
        let steps = parse_plan_steps(md);
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0].phase, "Phase 1: Analysis");
        assert_eq!(steps[0].title, "Read config files at src/config.ts");
        assert_eq!(steps[0].status, PlanStepStatus::Pending);
        assert_eq!(steps[1].status, PlanStepStatus::Completed);
        assert_eq!(steps[2].phase, "Phase 2: Implementation");
        assert_eq!(steps[2].index, 2);
    }

    #[test]
    fn test_plan_mode_state_roundtrip() {
        assert_eq!(PlanModeState::from_str("planning"), PlanModeState::Planning);
        assert_eq!(PlanModeState::from_str("review"), PlanModeState::Review);
        assert_eq!(
            PlanModeState::from_str("executing"),
            PlanModeState::Executing
        );
        assert_eq!(PlanModeState::from_str("paused"), PlanModeState::Paused);
        assert_eq!(
            PlanModeState::from_str("completed"),
            PlanModeState::Completed
        );
        assert_eq!(PlanModeState::from_str("off"), PlanModeState::Off);
        assert_eq!(PlanModeState::from_str("unknown"), PlanModeState::Off);
        assert_eq!(PlanModeState::Planning.as_str(), "planning");
        assert_eq!(PlanModeState::Review.as_str(), "review");
        assert_eq!(PlanModeState::Paused.as_str(), "paused");
        assert_eq!(PlanModeState::Completed.as_str(), "completed");
    }
}
