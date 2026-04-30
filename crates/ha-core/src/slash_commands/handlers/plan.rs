use crate::plan::{self, PlanModeState};
use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};

pub async fn handle_plan(
    db: &SessionDB,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session")?;

    match args.trim() {
        "" | "enter" => {
            if !plan::set_plan_state(sid, PlanModeState::Planning).await {
                return Err("Invalid plan mode transition to planning".to_string());
            }
            db.update_session_plan_mode(sid, PlanModeState::Planning)
                .map_err(|e| e.to_string())?;
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::EnterPlanMode),
            })
        }
        "exit" => {
            let plan_content = plan::load_plan_file(sid).ok().flatten();
            // Clean up git checkpoint if any
            if let Some(ref_name) = plan::get_checkpoint_ref(sid).await {
                plan::cleanup_checkpoint(&ref_name);
            }
            if !plan::set_plan_state(sid, PlanModeState::Off).await {
                return Err("Invalid plan mode transition to off".to_string());
            }
            db.update_session_plan_mode(sid, PlanModeState::Off)
                .map_err(|e| e.to_string())?;
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::ExitPlanMode { plan_content }),
            })
        }
        "approve" => {
            let plan_content = plan::load_plan_file(sid).ok().flatten();
            let previous_state = plan::get_plan_state(sid).await;
            let persisted_plan_mode = db
                .get_session(sid)
                .ok()
                .flatten()
                .map(|meta| meta.plan_mode);
            let checkpoint_exists = plan::get_checkpoint_ref(sid).await.is_some();
            let should_create_checkpoint = plan::should_create_execution_checkpoint(
                &PlanModeState::Executing,
                &previous_state,
                persisted_plan_mode,
                checkpoint_exists,
            );
            if !plan::set_plan_state(sid, PlanModeState::Executing).await {
                return Err("Invalid plan mode transition to executing".to_string());
            }
            db.update_session_plan_mode(sid, PlanModeState::Executing)
                .map_err(|e| e.to_string())?;
            // Create git checkpoint AFTER PlanMeta entry exists in the store
            if should_create_checkpoint {
                plan::create_checkpoint_for_session(sid).await;
            }
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
        "pause" => {
            let current = plan::get_plan_state(sid).await;
            if current != PlanModeState::Executing {
                return Err("Can only pause when plan is executing".to_string());
            }
            if !plan::set_plan_state(sid, PlanModeState::Paused).await {
                return Err("Invalid plan mode transition to paused".to_string());
            }
            db.update_session_plan_mode(sid, PlanModeState::Paused)
                .map_err(|e| e.to_string())?;
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::PausePlan),
            })
        }
        "resume" => {
            let current = plan::get_plan_state(sid).await;
            if current != PlanModeState::Paused {
                return Err("Can only resume when plan is paused".to_string());
            }
            if !plan::set_plan_state(sid, PlanModeState::Executing).await {
                return Err("Invalid plan mode transition to executing".to_string());
            }
            db.update_session_plan_mode(sid, PlanModeState::Executing)
                .map_err(|e| e.to_string())?;
            Ok(CommandResult {
                content: String::new(),
                action: Some(CommandAction::ResumePlan),
            })
        }
        _ => Err("Usage: /plan [enter|exit|approve|pause|resume|show]".to_string()),
    }
}
