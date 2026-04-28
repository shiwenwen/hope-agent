#[cfg(test)]
mod tests {
    use crate::plan::*;

    #[test]
    fn test_parse_plan_steps_from_headings() {
        let md = "\
## Context
Add theme switching without changing unrelated settings.

## Steps
### Step 1: Analysis
- Read config files at src/config.ts
- Analyze CSS variables in theme.css

### Step 2: Implementation
1. Add ThemeProvider component
2. Create toggle button

## Verification
1. Run typecheck";
        let steps = parse_plan_steps(md);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].phase, "Steps");
        assert_eq!(steps[0].title, "Step 1: Analysis");
        assert_eq!(steps[0].status, PlanStepStatus::Pending);
        assert_eq!(steps[1].phase, "Steps");
        assert_eq!(steps[1].title, "Step 2: Implementation");
        assert_eq!(steps[1].index, 1);
        assert_eq!(steps[2].phase, "Verification");
        assert_eq!(steps[2].title, "Verification");
    }

    #[test]
    fn test_parse_plan_steps_from_ordered_list() {
        let md = "\
## Context
Small change.

## Execution Plan
1. Update parser to read ordinary list items.
   - Keep nested bullets as details, not progress rows.
2. Update prompt wording to avoid checkboxes.

## Verification
1. Run cargo check";
        let steps = parse_plan_steps(md);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].phase, "Execution Plan");
        assert_eq!(steps[0].title, "Update parser to read ordinary list items.");
        assert_eq!(steps[1].title, "Update prompt wording to avoid checkboxes.");
        assert_eq!(steps[2].phase, "Verification");
        assert_eq!(steps[2].title, "Run cargo check");
    }

    #[test]
    fn test_parse_legacy_checklists_only_as_fallback() {
        let md = "\
- [ ] Read config files at src/config.ts
- [x] Analyze CSS variables in theme.css";
        let steps = parse_plan_steps(md);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].title, "Read config files at src/config.ts");
        assert_eq!(steps[0].status, PlanStepStatus::Pending);
        assert_eq!(steps[1].status, PlanStepStatus::Completed);
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
