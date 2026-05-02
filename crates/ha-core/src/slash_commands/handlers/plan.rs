use crate::plan::{self, PlanModeState, TransitionOutcome};
use crate::slash_commands::types::{CommandAction, CommandResult};

pub async fn handle_plan(session_id: Option<&str>, args: &str) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session")?;

    match args.trim() {
        "" | "enter" => {
            apply_transition(sid, PlanModeState::Planning, "slash_enter").await?;
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::EnterPlanMode),
            })
        }
        "exit" => {
            // `transition_state` handles cancel-subagent and checkpoint cleanup
            // automatically when target = Off, so the slash path stays aligned
            // with the GUI / HTTP set_plan_mode entry points.
            let plan_content = plan::load_plan_file(sid).ok().flatten();
            apply_transition(sid, PlanModeState::Off, "slash_exit").await?;
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::ExitPlanMode { plan_content }),
            })
        }
        "approve" => {
            // `transition_state` creates the git checkpoint on Executing.
            let plan_content = plan::load_plan_file(sid).ok().flatten();
            apply_transition(sid, PlanModeState::Executing, "slash_approve").await?;
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::ApprovePlan { plan_content }),
            })
        }
        "show" => {
            let plan_content = plan::load_plan_file(sid)
                .ok()
                .flatten()
                .unwrap_or_else(|| "No plan found for this session.".to_string());
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::ShowPlan { plan_content }),
            })
        }
        _ => Err("Usage: /plan [enter|exit|approve|show]".to_string()),
    }
}

async fn apply_transition(
    sid: &str,
    target: PlanModeState,
    reason: &'static str,
) -> Result<(), String> {
    match plan::transition_state(sid, target, reason).await {
        Ok(TransitionOutcome::Applied) => Ok(()),
        Ok(TransitionOutcome::Rejected) => Err(format!(
            "Invalid plan mode transition to '{}'",
            target.as_str()
        )),
        Err(e) => Err(e.to_string()),
    }
}
