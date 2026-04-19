use std::sync::Arc;

use super::types::*;
use crate::session::SessionDB;

/// Clean up orphaned teams on startup.
/// - Members with status=working whose subagent runs are gone → mark Error
/// - Teams with all members in terminal state → leave Active for user to decide
pub fn cleanup_orphan_teams(db: &Arc<SessionDB>) {
    let teams = match db.list_active_teams() {
        Ok(t) => t,
        Err(e) => {
            app_warn!("team", "cleanup", "Failed to list active teams: {}", e);
            return;
        }
    };

    for team in &teams {
        let members = match db.list_team_members(&team.team_id) {
            Ok(m) => m,
            Err(_) => continue,
        };

        for member in &members {
            if !member.status.is_active() {
                continue;
            }

            // Check if the underlying subagent run still exists and is active
            if let Some(ref run_id) = member.run_id {
                match db.get_subagent_run(run_id) {
                    Ok(Some(run)) => {
                        if run.status.is_terminal() {
                            // Subagent finished but member not updated — sync status
                            let new_status = match run.status {
                                crate::subagent::SubagentStatus::Completed => {
                                    MemberStatus::Completed
                                }
                                crate::subagent::SubagentStatus::Killed => MemberStatus::Killed,
                                _ => MemberStatus::Error,
                            };
                            let _ = db.update_team_member_status(&member.member_id, &new_status);
                            app_info!(
                                "team",
                                "cleanup",
                                "Synced member {} status to {:?} (run {})",
                                member.name,
                                new_status,
                                run_id
                            );
                        }
                    }
                    Ok(None) => {
                        // Run record gone — mark member as error
                        let _ =
                            db.update_team_member_status(&member.member_id, &MemberStatus::Error);
                        app_warn!(
                            "team",
                            "cleanup",
                            "Orphaned team member {} (run {} missing), marked Error",
                            member.name,
                            run_id
                        );
                    }
                    Err(_) => {}
                }
            } else {
                // No run_id but status is active — shouldn't happen, reset to idle
                let _ = db.update_team_member_status(&member.member_id, &MemberStatus::Idle);
            }
        }
    }

    if !teams.is_empty() {
        app_info!(
            "team",
            "cleanup",
            "Checked {} active team(s) for orphaned members",
            teams.len()
        );
    }
}
