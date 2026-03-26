use crate::plan::{self, PlanModeState};
use crate::session::SessionDB;
use crate::slash_commands::types::{CommandResult, CommandAction};

pub async fn handle_plan(
    db: &SessionDB,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session")?;

    match args.trim() {
        "" | "enter" => {
            plan::set_plan_state(sid, PlanModeState::Planning).await;
            db.update_session_plan_mode(sid, "planning").map_err(|e| e.to_string())?;
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::EnterPlanMode),
            })
        }
        "exit" => {
            let plan_content = plan::load_plan_file(sid).ok().flatten();
            plan::set_plan_state(sid, PlanModeState::Off).await;
            db.update_session_plan_mode(sid, "off").map_err(|e| e.to_string())?;
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::ExitPlanMode { plan_content }),
            })
        }
        "approve" => {
            let plan_content = plan::load_plan_file(sid).ok().flatten();
            plan::set_plan_state(sid, PlanModeState::Executing).await;
            db.update_session_plan_mode(sid, "executing").map_err(|e| e.to_string())?;
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
