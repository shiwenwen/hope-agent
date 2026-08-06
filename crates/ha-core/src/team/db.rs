use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use super::types::*;
use crate::session::SessionDB;

const TEAM_NOT_CONTROLLED: &str = "Team was not found or is not controlled by the current session";

enum AuthorizedTeamActor {
    Lead,
    Member { member_id: String, name: String },
}

impl SessionDB {
    fn authorize_active_team_actor(
        conn: &rusqlite::Connection,
        team_id: &str,
        session_id: &str,
    ) -> Result<AuthorizedTeamActor> {
        let (status, lead_session_id): (String, String) = conn
            .query_row(
                "SELECT status, lead_session_id FROM teams WHERE team_id = ?1",
                params![team_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!(TEAM_NOT_CONTROLLED),
                other => other.into(),
            })?;
        if status != TeamStatus::Active.as_str() {
            anyhow::bail!(TEAM_NOT_CONTROLLED);
        }
        if lead_session_id == session_id {
            return Ok(AuthorizedTeamActor::Lead);
        }
        conn.query_row(
            "SELECT m.member_id, m.name
             FROM team_members m
             JOIN subagent_runs r ON r.run_id = m.run_id
             WHERE m.team_id = ?1 AND m.session_id = ?2
               AND m.status IN ('idle', 'working')
               AND r.child_session_id = m.session_id
               AND r.parent_session_id = ?3
               AND r.owner_kind = 'team' AND r.owner_id = ?1
               AND r.status IN ('queued', 'spawning', 'running')
             LIMIT 1",
            params![team_id, session_id, lead_session_id],
            |row| {
                Ok(AuthorizedTeamActor::Member {
                    member_id: row.get(0)?,
                    name: row.get(1)?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!(TEAM_NOT_CONTROLLED),
            other => other.into(),
        })
    }

    // ── Teams CRUD ──────────────────────────────────────────────

    pub fn insert_team(&self, team: &Team) -> Result<()> {
        {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let config_json = serde_json::to_string(&team.config)?;
            conn.execute(
                "INSERT INTO teams (team_id, name, description, lead_session_id, lead_agent_id,
                 status, created_at, updated_at, template_id, config_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    team.team_id,
                    team.name,
                    team.description,
                    team.lead_session_id,
                    team.lead_agent_id,
                    team.status.as_str(),
                    team.created_at,
                    team.updated_at,
                    team.template_id,
                    config_json,
                ],
            )?;
        }
        crate::eval_context::record_lifecycle_event(
            Some(&team.lead_session_id),
            "team",
            "team.created",
            Some(&team.team_id),
            team.status.as_str(),
            0,
        );
        Ok(())
    }

    pub fn get_team(&self, team_id: &str) -> Result<Option<Team>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT team_id, name, description, lead_session_id, lead_agent_id,
                    status, created_at, updated_at, template_id, config_json
             FROM teams WHERE team_id = ?1",
        )?;
        let result = stmt.query_row(params![team_id], Self::row_to_team);
        match result {
            Ok(team) => Ok(Some(team)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_teams_by_session(&self, session_id: &str) -> Result<Vec<Team>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT team_id, name, description, lead_session_id, lead_agent_id,
                    status, created_at, updated_at, template_id, config_json
             FROM teams WHERE lead_session_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![session_id], Self::row_to_team)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_active_teams(&self) -> Result<Vec<Team>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT team_id, name, description, lead_session_id, lead_agent_id,
                    status, created_at, updated_at, template_id, config_json
             FROM teams WHERE status = 'active' ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], Self::row_to_team)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_team_status(&self, team_id: &str, status: &TeamStatus) -> Result<()> {
        let lead_session_id = {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let lead_session_id = conn
                .query_row(
                    "SELECT lead_session_id FROM teams WHERE team_id = ?1",
                    params![team_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            conn.execute(
                "UPDATE teams SET status = ?1, updated_at = datetime('now') WHERE team_id = ?2",
                params![status.as_str(), team_id],
            )?;
            lead_session_id
        };
        crate::eval_context::record_lifecycle_event(
            lead_session_id.as_deref(),
            "team",
            "team.transition",
            Some(team_id),
            status.as_str(),
            0,
        );
        Ok(())
    }

    /// Atomically fence an Active team before cancellation and snapshot the
    /// exact member run ids that existed at the transition boundary. The
    /// durable Paused/member state is committed before callers signal any
    /// process-local cancellation tokens, so queued promotion and old member
    /// sessions are denied immediately after this method returns.
    pub fn pause_active_team_and_snapshot_runs(
        &self,
        team_id: &str,
    ) -> Result<(usize, Vec<String>)> {
        let (lead_session_id, paused_count, run_ids) = {
            let mut conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let tx = conn.transaction()?;
            let (status, lead_session_id): (String, String) = tx
                .query_row(
                    "SELECT status, lead_session_id FROM teams WHERE team_id = ?1",
                    params![team_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("Team not found"),
                    other => other.into(),
                })?;
            if status != TeamStatus::Active.as_str() {
                anyhow::bail!("Team is not active");
            }

            // Reconcile stale roster rows against the durable subagent truth
            // before selecting pause candidates. A member whose attempt had
            // already terminated must retain that terminal meaning instead of
            // being converted to Paused and accidentally restarted later.
            tx.execute(
                "UPDATE team_members
                 SET status = CASE
                       WHEN run_id IN (
                           SELECT run_id FROM subagent_runs WHERE status = 'completed'
                       ) THEN 'completed'
                       WHEN run_id IN (
                           SELECT run_id FROM subagent_runs WHERE status = 'killed'
                       ) THEN 'killed'
                       ELSE 'error'
                     END,
                     last_active_at = datetime('now')
                 WHERE team_id = ?1 AND status IN ('idle', 'working')
                   AND run_id IN (
                       SELECT run_id FROM subagent_runs
                       WHERE status IN ('completed', 'killed', 'error', 'timeout', 'interrupted')
                   )",
                params![team_id],
            )?;

            let run_ids = {
                let mut stmt = tx.prepare(
                    "SELECT run_id FROM team_members
                     WHERE team_id = ?1 AND status IN ('idle', 'working') AND run_id IS NOT NULL",
                )?;
                let rows = stmt.query_map(params![team_id], |row| row.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let paused_count = tx.execute(
                "UPDATE team_members SET status = 'paused', last_active_at = datetime('now')
                 WHERE team_id = ?1 AND status IN ('idle', 'working')",
                params![team_id],
            )?;
            let transitioned = tx.execute(
                "UPDATE teams SET status = 'paused', updated_at = datetime('now')
                 WHERE team_id = ?1 AND status = 'active'",
                params![team_id],
            )?;
            if transitioned != 1 {
                anyhow::bail!("Team is not active");
            }
            tx.commit()?;
            (lead_session_id, paused_count, run_ids)
        };
        crate::eval_context::record_lifecycle_event(
            Some(&lead_session_id),
            "team",
            "team.transition",
            Some(team_id),
            TeamStatus::Paused.as_str(),
            0,
        );
        Ok((paused_count, run_ids))
    }

    /// Inspect a Paused team for one resume operation and return the original
    /// paused member rows split by old-attempt eligibility. If any old attempt
    /// is still non-terminal, the whole team remains Paused and no claim is
    /// made. Otherwise the conditional status update serializes concurrent
    /// resume/dissolve calls; callers must reuse these member ids rather than
    /// inserting replacement roster rows.
    pub fn begin_resume_team(
        &self,
        team_id: &str,
    ) -> Result<(
        Team,
        Vec<TeamMember>,
        Vec<(TeamMember, String, String)>,
        Vec<TeamMember>,
    )> {
        let (team, eligible_members, pending_members, completed_members, claimed) = {
            let mut conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let tx = conn.transaction()?;
            let mut team = tx
                .query_row(
                    "SELECT team_id, name, description, lead_session_id, lead_agent_id,
                            status, created_at, updated_at, template_id, config_json
                     FROM teams WHERE team_id = ?1",
                    params![team_id],
                    Self::row_to_team,
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("Team not found"),
                    other => other.into(),
                })?;
            if team.status != TeamStatus::Paused {
                anyhow::bail!("Team is not paused");
            }

            // Cancellation is cooperative: an attempt can commit successful
            // completion after pause has durably marked its roster row Paused.
            // Success wins that race. Reconcile it before selecting resume
            // candidates so an explicit resume cannot duplicate already-
            // successful external side effects.
            let mut completed_members = {
                let mut stmt = tx.prepare(
                    "SELECT m.member_id, m.team_id, m.name, m.agent_id, m.role, m.status,
                            m.run_id, m.session_id, m.color, m.current_task_id, m.model_override,
                            m.role_description, m.joined_at, m.last_active_at,
                            m.input_tokens, m.output_tokens
                       FROM team_members m
                       JOIN subagent_runs r ON r.run_id = m.run_id
                      WHERE m.team_id = ?1 AND m.status = 'paused'
                        AND r.status = 'completed'
                      ORDER BY m.joined_at ASC",
                )?;
                let rows = stmt.query_map(params![team_id], Self::row_to_team_member)?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            if !completed_members.is_empty() {
                tx.execute(
                    "UPDATE team_members
                        SET status = 'completed', last_active_at = datetime('now')
                      WHERE team_id = ?1 AND status = 'paused'
                        AND run_id IN (
                            SELECT run_id FROM subagent_runs WHERE status = 'completed'
                        )",
                    params![team_id],
                )?;
                for member in &mut completed_members {
                    member.status = MemberStatus::Completed;
                }
            }
            let members_with_run_status = {
                let mut stmt = tx.prepare(
                    "SELECT m.member_id, m.team_id, m.name, m.agent_id, m.role, m.status,
                            m.run_id, m.session_id, m.color, m.current_task_id, m.model_override,
                            m.role_description, m.joined_at, m.last_active_at,
                            m.input_tokens, m.output_tokens,
                            r.status
                     FROM team_members m
                     LEFT JOIN subagent_runs r ON r.run_id = m.run_id
                     WHERE m.team_id = ?1 AND m.status = 'paused'
                     ORDER BY m.joined_at ASC",
                )?;
                let rows = stmt.query_map(params![team_id], |row| {
                    Ok((
                        Self::row_to_team_member(row)?,
                        row.get::<_, Option<String>>(16)?,
                    ))
                })?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            if members_with_run_status.is_empty() {
                // A successful no-op must remain stable after the first resume
                // reconciles Paused -> Completed. Re-read the whole roster in
                // this same transaction: only a non-empty, entirely Completed
                // roster is an idempotent already-complete outcome. An empty
                // roster or any Error/Killed/Idle/Working member must not be
                // mistaken for successful completion merely because no Paused
                // row remains.
                let roster = {
                    let mut stmt = tx.prepare(
                        "SELECT member_id, team_id, name, agent_id, role, status,
                                run_id, session_id, color, current_task_id, model_override,
                                role_description, joined_at, last_active_at,
                                input_tokens, output_tokens
                           FROM team_members
                          WHERE team_id = ?1
                          ORDER BY joined_at ASC, member_id ASC",
                    )?;
                    let rows = stmt.query_map(params![team_id], Self::row_to_team_member)?;
                    rows.collect::<Result<Vec<_>, _>>()?
                };
                if roster.is_empty()
                    || roster
                        .iter()
                        .any(|member| member.status != MemberStatus::Completed)
                {
                    anyhow::bail!("Team has no paused members to resume");
                }
                // Return the complete roster, not only rows reconciled in this
                // invocation, so refreshes/retries produce the same wire result.
                completed_members = roster;
            }
            let mut eligible_members = Vec::new();
            let mut pending_members = Vec::new();
            for (member, run_status) in members_with_run_status {
                match (member.run_id.as_deref(), run_status.as_deref()) {
                    (None, _) => eligible_members.push(member),
                    (
                        Some(_),
                        Some("completed" | "killed" | "error" | "timeout" | "interrupted"),
                    ) => {
                        eligible_members.push(member);
                    }
                    (Some(_), Some(status)) => pending_members.push((
                        member,
                        super::RESUME_BLOCK_OLD_ATTEMPT_ACTIVE.to_string(),
                        status.to_string(),
                    )),
                    (Some(_), None) => pending_members.push((
                        member,
                        super::RESUME_BLOCK_OLD_ATTEMPT_UNKNOWN.to_string(),
                        super::RESUME_BLOCK_MISSING_RUN_RECORD.to_string(),
                    )),
                }
            }
            let claimed = !eligible_members.is_empty() && pending_members.is_empty();
            if claimed {
                let transitioned = tx.execute(
                    "UPDATE teams SET status = 'active', updated_at = datetime('now')
                     WHERE team_id = ?1 AND status = 'paused'",
                    params![team_id],
                )?;
                if transitioned != 1 {
                    anyhow::bail!("Team is not paused");
                }
                team.status = TeamStatus::Active;
            }
            tx.commit()?;
            (
                team,
                eligible_members,
                pending_members,
                completed_members,
                claimed,
            )
        };
        if claimed {
            crate::eval_context::record_lifecycle_event(
                Some(&team.lead_session_id),
                "team",
                "team.transition",
                Some(team_id),
                TeamStatus::Active.as_str(),
                0,
            );
        }
        Ok((team, eligible_members, pending_members, completed_members))
    }

    /// Final pre-spawn gate for resume. Returning a blocker means the old
    /// attempt is still non-terminal and a fresh attempt would run in parallel.
    /// A member with no run id is eligible. A non-null run id whose durable row
    /// is missing is blocked as unknown: absence cannot prove termination and
    /// therefore must fail closed.
    pub fn team_member_resume_blocker(
        &self,
        team_id: &str,
        member_id: &str,
        expected_run_id: Option<&str>,
    ) -> Result<Option<(String, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let (team_status, member_status, current_run_id): (String, String, Option<String>) = conn
            .query_row(
                "SELECT t.status, m.status, m.run_id
                 FROM team_members m
                 JOIN teams t ON t.team_id = m.team_id
                 WHERE m.team_id = ?1 AND m.member_id = ?2",
                params![team_id, member_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("Team member not found"),
                other => other.into(),
            })?;
        if team_status != TeamStatus::Active.as_str()
            || member_status != MemberStatus::Paused.as_str()
            || current_run_id.as_deref() != expected_run_id
        {
            anyhow::bail!("Team/member state changed before resume launch");
        }
        let Some(run_id) = current_run_id else {
            return Ok(None);
        };
        let run_status = conn
            .query_row(
                "SELECT status FROM subagent_runs WHERE run_id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(match run_status.as_deref() {
            None => Some((
                super::RESUME_BLOCK_OLD_ATTEMPT_UNKNOWN.to_string(),
                super::RESUME_BLOCK_MISSING_RUN_RECORD.to_string(),
            )),
            Some("completed" | "killed" | "error" | "timeout" | "interrupted") => None,
            Some(status) => Some((
                super::RESUME_BLOCK_OLD_ATTEMPT_ACTIVE.to_string(),
                status.to_string(),
            )),
        })
    }

    /// Attach a fresh immutable subagent attempt to an existing roster member.
    /// Both the team and member status are part of the conditional write, so a
    /// concurrent pause/dissolve/remove revokes the launch before it can become
    /// the member's live capability.
    pub fn activate_team_member_attempt(
        &self,
        team_id: &str,
        member_id: &str,
        expected_status: &MemberStatus,
        expected_run_id: Option<&str>,
        expected_session_id: Option<&str>,
        run_id: &str,
        session_id: &str,
    ) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let updated = conn.execute(
            "UPDATE team_members
             SET run_id = ?1, session_id = ?2, status = 'working',
                 last_active_at = datetime('now')
             WHERE member_id = ?3 AND team_id = ?4 AND status = ?5
               AND run_id IS ?6 AND session_id IS ?7
               AND EXISTS (
                   SELECT 1 FROM teams
                   WHERE teams.team_id = ?4 AND teams.status = 'active'
               )
               AND EXISTS (
                   SELECT 1
                   FROM subagent_runs r
                   JOIN teams t ON t.team_id = ?4
                   WHERE r.run_id = ?1
                     AND r.child_session_id = ?2
                     AND r.parent_session_id = t.lead_session_id
                     AND r.owner_kind = 'team' AND r.owner_id = ?4
                     AND r.status IN ('queued', 'spawning')
               )",
            params![
                run_id,
                session_id,
                member_id,
                team_id,
                expected_status.as_str(),
                expected_run_id,
                expected_session_id,
            ],
        )?;
        Ok(updated == 1)
    }

    /// Final execution claim for a prepared Team attempt. This is deliberately
    /// one conditional UPDATE rather than a status read followed by a generic
    /// subagent launch: the run becomes Running only while the exact roster
    /// attachment and Active Team capability still exist. Therefore this write
    /// and pause/dissolve have a deterministic winner.
    pub fn claim_team_member_attempt_launch(
        &self,
        team_id: &str,
        member_id: &str,
        run_id: &str,
        session_id: &str,
        expected_run_status: &crate::subagent::SubagentStatus,
    ) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE subagent_runs
                SET status = 'running', last_heartbeat_at = ?1
              WHERE run_id = ?2 AND child_session_id = ?3 AND status = ?4
                AND owner_kind = 'team' AND owner_id = ?5
                AND EXISTS (
                    SELECT 1
                      FROM team_members m
                      JOIN teams t ON t.team_id = m.team_id
                     WHERE m.team_id = ?5 AND m.member_id = ?6
                       AND m.status = 'working'
                       AND m.run_id = ?2 AND m.session_id = ?3
                       AND t.status = 'active'
                )
                AND EXISTS (
                    SELECT 1 FROM subagent_threads st
                     WHERE st.thread_id = subagent_runs.child_session_id
                       AND st.current_run_id = subagent_runs.run_id
                       AND st.lease_epoch = subagent_runs.lease_epoch
                )",
            params![
                chrono::Utc::now().to_rfc3339(),
                run_id,
                session_id,
                expected_run_status.as_str(),
                team_id,
                member_id,
            ],
        )?;
        Ok(changed == 1)
    }

    /// Restore a roster row when an attached prepared attempt could not be
    /// scheduled. The exact new run/session predicate prevents this cleanup
    /// from undoing a concurrent pause, dissolve, removal, or newer attempt.
    #[allow(clippy::too_many_arguments)]
    pub fn restore_team_member_after_unlaunched_attempt(
        &self,
        team_id: &str,
        member_id: &str,
        attempted_run_id: &str,
        attempted_session_id: &str,
        previous_status: &MemberStatus,
        previous_run_id: Option<&str>,
        previous_session_id: Option<&str>,
    ) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let changed = conn.execute(
            "UPDATE team_members
                SET status = ?1, run_id = ?2, session_id = ?3,
                    last_active_at = datetime('now')
              WHERE team_id = ?4 AND member_id = ?5 AND status = 'working'
                AND run_id = ?6 AND session_id = ?7
                AND EXISTS (
                    SELECT 1 FROM teams WHERE team_id = ?4 AND status = 'active'
                )",
            params![
                previous_status.as_str(),
                previous_run_id,
                previous_session_id,
                team_id,
                member_id,
                attempted_run_id,
                attempted_session_id,
            ],
        )?;
        Ok(changed == 1)
    }

    /// If a resume operation launched no members, return the claimed team to
    /// Paused only while it is still Active and no concurrent operation made a
    /// member live.
    pub fn restore_paused_if_no_active_members(&self, team_id: &str) -> Result<bool> {
        let (lead_session_id, restored) = {
            let conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let lead_session_id = conn
                .query_row(
                    "SELECT lead_session_id FROM teams WHERE team_id = ?1",
                    params![team_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            let restored = conn.execute(
                "UPDATE teams SET status = 'paused', updated_at = datetime('now')
                 WHERE team_id = ?1 AND status = 'active'
                   AND NOT EXISTS (
                       SELECT 1 FROM team_members
                       WHERE team_id = ?1 AND status IN ('idle', 'working')
                   )",
                params![team_id],
            )? == 1;
            (lead_session_id, restored)
        };
        if restored {
            crate::eval_context::record_lifecycle_event(
                lead_session_id.as_deref(),
                "team",
                "team.transition",
                Some(team_id),
                TeamStatus::Paused.as_str(),
                0,
            );
        }
        Ok(restored)
    }

    /// Atomically revoke one roster member from an Active team and return the
    /// exact run id to cancel after the durable status change commits.
    pub fn remove_active_team_member_and_snapshot_run(
        &self,
        team_id: &str,
        member_id: &str,
    ) -> Result<(TeamMember, Option<String>)> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let team_status: String = tx
            .query_row(
                "SELECT status FROM teams WHERE team_id = ?1",
                params![team_id],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("Team not found"),
                other => other.into(),
            })?;
        if team_status != TeamStatus::Active.as_str() {
            anyhow::bail!("Team is not active");
        }
        let member = tx
            .query_row(
                "SELECT member_id, team_id, name, agent_id, role, status,
                        run_id, session_id, color, current_task_id, model_override,
                        role_description, joined_at, last_active_at, input_tokens, output_tokens
                 FROM team_members WHERE member_id = ?1 AND team_id = ?2",
                params![member_id, team_id],
                Self::row_to_team_member,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("Member not found"),
                other => other.into(),
            })?;
        let run_id = member
            .status
            .is_active()
            .then(|| member.run_id.clone())
            .flatten();
        tx.execute(
            "UPDATE team_members SET status = 'killed', last_active_at = datetime('now')
             WHERE member_id = ?1 AND team_id = ?2",
            params![member_id, team_id],
        )?;
        tx.commit()?;
        Ok((member, run_id))
    }

    /// Atomically revoke a live team and every non-terminal roster capability,
    /// then return the exact run ids for canonical best-effort cancellation.
    pub fn dissolve_team_and_snapshot_runs(&self, team_id: &str) -> Result<(Team, Vec<String>)> {
        let (team, run_ids) = {
            let mut conn = self
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let tx = conn.transaction()?;
            let team = tx
                .query_row(
                    "SELECT team_id, name, description, lead_session_id, lead_agent_id,
                            status, created_at, updated_at, template_id, config_json
                     FROM teams WHERE team_id = ?1",
                    params![team_id],
                    Self::row_to_team,
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => anyhow::anyhow!("Team not found"),
                    other => other.into(),
                })?;
            if !matches!(team.status, TeamStatus::Active | TeamStatus::Paused) {
                anyhow::bail!("Team is already dissolved");
            }
            let run_ids = {
                let mut stmt = tx.prepare(
                    "SELECT run_id FROM team_members
                     WHERE team_id = ?1 AND status IN ('idle', 'working', 'paused')
                       AND run_id IS NOT NULL",
                )?;
                let rows = stmt.query_map(params![team_id], |row| row.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            tx.execute(
                "UPDATE team_members SET status = 'killed', last_active_at = datetime('now')
                 WHERE team_id = ?1 AND status IN ('idle', 'working', 'paused')",
                params![team_id],
            )?;
            let transitioned = tx.execute(
                "UPDATE teams SET status = 'dissolved', updated_at = datetime('now')
                 WHERE team_id = ?1 AND status IN ('active', 'paused')",
                params![team_id],
            )?;
            if transitioned != 1 {
                anyhow::bail!("Team is already dissolved");
            }
            tx.commit()?;
            (team, run_ids)
        };
        crate::eval_context::record_lifecycle_event(
            Some(&team.lead_session_id),
            "team",
            "team.transition",
            Some(team_id),
            TeamStatus::Dissolved.as_str(),
            0,
        );
        Ok((team, run_ids))
    }

    pub fn count_active_teams_for_agent(&self, agent_id: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM teams WHERE lead_agent_id = ?1 AND status = 'active'",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Count active teams where the Agent is either the lead or a member.
    /// Lifecycle deletion uses the broader relation so a worker cannot vanish
    /// while its team is still executing.
    pub fn count_active_teams_involving_agent(&self, agent_id: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT t.team_id)
             FROM teams t
             LEFT JOIN team_members m ON m.team_id=t.team_id
             WHERE t.status='active' AND (t.lead_agent_id=?1 OR m.agent_id=?1)",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    // ── Team Members CRUD ───────────────────────────────────────

    pub fn insert_team_member(&self, member: &TeamMember) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let inserted = conn.execute(
            "INSERT INTO team_members (member_id, team_id, name, agent_id, role, status,
             run_id, session_id, color, current_task_id, model_override, role_description,
             joined_at, last_active_at, input_tokens, output_tokens)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
             WHERE EXISTS (
                 SELECT 1 FROM teams WHERE team_id = ?2 AND status = 'active'
             )",
            params![
                member.member_id,
                member.team_id,
                member.name,
                member.agent_id,
                member.role.as_str(),
                member.status.as_str(),
                member.run_id,
                member.session_id,
                member.color,
                member.current_task_id,
                member.model_override,
                member.role_description,
                member.joined_at,
                member.last_active_at,
                member.input_tokens.unwrap_or(0) as i64,
                member.output_tokens.unwrap_or(0) as i64,
            ],
        )?;
        if inserted != 1 {
            anyhow::bail!("Team is not active");
        }
        Ok(())
    }

    pub fn get_team_member(&self, member_id: &str) -> Result<Option<TeamMember>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let result = conn.query_row(
            "SELECT member_id, team_id, name, agent_id, role, status,
                    run_id, session_id, color, current_task_id, model_override, role_description,
                    joined_at, last_active_at, input_tokens, output_tokens
             FROM team_members WHERE member_id = ?1",
            params![member_id],
            Self::row_to_team_member,
        );
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_team_members(&self, team_id: &str) -> Result<Vec<TeamMember>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT member_id, team_id, name, agent_id, role, status,
                    run_id, session_id, color, current_task_id, model_override, role_description,
                    joined_at, last_active_at, input_tokens, output_tokens
             FROM team_members WHERE team_id = ?1 ORDER BY joined_at ASC",
        )?;
        let rows = stmt.query_map(params![team_id], Self::row_to_team_member)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_team_member_status(&self, member_id: &str, status: &MemberStatus) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE team_members SET status = ?1, last_active_at = datetime('now')
             WHERE member_id = ?2",
            params![status.as_str(), member_id],
        )?;
        Ok(())
    }

    pub fn update_team_member_run(
        &self,
        member_id: &str,
        run_id: &str,
        session_id: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE team_members SET run_id = ?1, session_id = ?2, status = 'working',
             last_active_at = datetime('now') WHERE member_id = ?3",
            params![run_id, session_id, member_id],
        )?;
        Ok(())
    }

    pub fn update_team_member_tokens(
        &self,
        member_id: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE team_members SET input_tokens = ?1, output_tokens = ?2 WHERE member_id = ?3",
            params![input_tokens as i64, output_tokens as i64, member_id],
        )?;
        Ok(())
    }

    pub fn update_team_member_task(&self, member_id: &str, task_id: Option<i64>) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE team_members SET current_task_id = ?1 WHERE member_id = ?2",
            params![task_id, member_id],
        )?;
        Ok(())
    }

    pub fn delete_team_member(&self, member_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM team_members WHERE member_id = ?1",
            params![member_id],
        )?;
        Ok(())
    }

    pub fn find_team_member_by_name(
        &self,
        team_id: &str,
        name: &str,
    ) -> Result<Option<TeamMember>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let result = conn.query_row(
            "SELECT member_id, team_id, name, agent_id, role, status,
                    run_id, session_id, color, current_task_id, model_override, role_description,
                    joined_at, last_active_at, input_tokens, output_tokens
             FROM team_members WHERE team_id = ?1 AND name = ?2",
            params![team_id, name],
            Self::row_to_team_member,
        );
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn find_team_member_by_run_id(&self, run_id: &str) -> Result<Option<TeamMember>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let result = conn.query_row(
            "SELECT member_id, team_id, name, agent_id, role, status,
                    run_id, session_id, color, current_task_id, model_override, role_description,
                    joined_at, last_active_at, input_tokens, output_tokens
             FROM team_members WHERE run_id = ?1",
            params![run_id],
            Self::row_to_team_member,
        );
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn row_to_team(row: &rusqlite::Row) -> rusqlite::Result<Team> {
        let config_json: String = row.get(9)?;
        Ok(Team {
            team_id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            lead_session_id: row.get(3)?,
            lead_agent_id: row.get(4)?,
            status: TeamStatus::from_str(&row.get::<_, String>(5)?),
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            template_id: row.get(8)?,
            config: serde_json::from_str(&config_json).unwrap_or_default(),
        })
    }

    fn row_to_team_member(row: &rusqlite::Row) -> rusqlite::Result<TeamMember> {
        let input: i64 = row.get(14)?;
        let output: i64 = row.get(15)?;
        Ok(TeamMember {
            member_id: row.get(0)?,
            team_id: row.get(1)?,
            name: row.get(2)?,
            agent_id: row.get(3)?,
            role: MemberRole::from_str(&row.get::<_, String>(4)?),
            status: MemberStatus::from_str(&row.get::<_, String>(5)?),
            run_id: row.get(6)?,
            session_id: row.get(7)?,
            color: row.get(8)?,
            current_task_id: row.get(9)?,
            model_override: row.get(10)?,
            role_description: row.get(11)?,
            joined_at: row.get(12)?,
            last_active_at: row.get(13)?,
            input_tokens: Some(input as u64),
            output_tokens: Some(output as u64),
        })
    }

    // ── Team Messages ───────────────────────────────────────────

    pub fn insert_team_message(&self, msg: &TeamMessage) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO team_messages (message_id, team_id, from_member_id, to_member_id,
             content, message_type, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                msg.message_id,
                msg.team_id,
                msg.from_member_id,
                msg.to_member_id,
                msg.content,
                msg.message_type.as_str(),
                msg.timestamp,
            ],
        )?;
        Ok(())
    }

    /// Model-plane Team message write with authorization, recipient
    /// resolution, persistence, and delivery-target snapshot under one SQLite
    /// transaction. A member removed/paused before the transaction cannot
    /// write; a concurrent revocation after commit cannot undo the already
    /// persisted message but will cancel its snapshotted run separately.
    pub fn insert_authorized_team_message(
        &self,
        team_id: &str,
        session_id: &str,
        to: Option<&str>,
        content: &str,
    ) -> Result<(TeamMessage, String, Vec<String>)> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let actor = Self::authorize_active_team_actor(&tx, team_id, session_id)?;
        let (from_member_id, sender_name) = match actor {
            AuthorizedTeamActor::Lead => ("*lead*".to_string(), "lead".to_string()),
            AuthorizedTeamActor::Member { member_id, name } => (member_id, name),
        };
        let to_member_id = match to.filter(|value| *value != "*") {
            Some(target) => Some(
                tx.query_row(
                    "SELECT member_id FROM team_members
                     WHERE team_id = ?1 AND (member_id = ?2 OR name = ?2)
                       AND status IN ('idle', 'working')",
                    params![team_id, target],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        anyhow::anyhow!("Message recipient is not an active team member")
                    }
                    other => other.into(),
                })?,
            ),
            None => None,
        };
        let msg = TeamMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            from_member_id: from_member_id.clone(),
            to_member_id: to_member_id.clone(),
            content: content.to_string(),
            message_type: TeamMessageType::Chat,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        tx.execute(
            "INSERT INTO team_messages (message_id, team_id, from_member_id, to_member_id,
             content, message_type, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                msg.message_id,
                msg.team_id,
                msg.from_member_id,
                msg.to_member_id,
                msg.content,
                msg.message_type.as_str(),
                msg.timestamp,
            ],
        )?;
        let recipient_runs = {
            let mut stmt = tx.prepare(
                "SELECT m.run_id
                 FROM team_members m
                 JOIN subagent_runs r ON r.run_id = m.run_id
                 WHERE m.team_id = ?1 AND m.status IN ('idle', 'working')
                   AND m.run_id IS NOT NULL
                   AND (?2 IS NULL OR m.member_id = ?2)
                   AND m.member_id != ?3
                   AND r.child_session_id = m.session_id
                   AND r.parent_session_id = (
                       SELECT lead_session_id FROM teams WHERE team_id = ?1
                   )
                   AND r.owner_kind = 'team' AND r.owner_id = ?1
                   AND r.status IN ('queued', 'spawning', 'running')",
            )?;
            let rows = stmt.query_map(params![team_id, to_member_id, from_member_id], |row| {
                row.get::<_, String>(0)
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        tx.commit()?;
        Ok((msg, sender_name, recipient_runs))
    }

    /// Load the latest `limit` team messages in ASC order, with a `has_more`
    /// flag indicating whether older messages exist beyond the window.
    ///
    /// Uses composite cursor `(timestamp, message_id)` so same-millisecond
    /// inserts are paginated deterministically. `timestamp` is RFC3339 so
    /// lexicographic comparison matches chronological order.
    pub fn list_team_messages_latest(
        &self,
        team_id: &str,
        limit: u32,
    ) -> Result<(Vec<TeamMessage>, bool)> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT message_id, team_id, from_member_id, to_member_id,
                    content, message_type, timestamp
             FROM team_messages WHERE team_id = ?1
             ORDER BY timestamp DESC, message_id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![team_id, limit as i64], Self::row_to_team_message)?;
        let mut messages: Vec<TeamMessage> = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        messages.reverse(); // oldest first

        let has_more = match messages.first() {
            Some(first) => Self::has_team_messages_before(&conn, team_id, first)?,
            None => false,
        };

        Ok((messages, has_more))
    }

    /// Load messages strictly older than the given cursor in ASC order, with
    /// `has_more`. Cursor is `(before_timestamp, before_message_id)` — the
    /// first message currently in view (client-maintained oldest cursor).
    pub fn list_team_messages_before(
        &self,
        team_id: &str,
        before_timestamp: &str,
        before_message_id: &str,
        limit: u32,
    ) -> Result<(Vec<TeamMessage>, bool)> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT message_id, team_id, from_member_id, to_member_id,
                    content, message_type, timestamp
             FROM team_messages
             WHERE team_id = ?1
               AND (timestamp < ?2
                    OR (timestamp = ?2 AND message_id < ?3))
             ORDER BY timestamp DESC, message_id DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![team_id, before_timestamp, before_message_id, limit as i64],
            Self::row_to_team_message,
        )?;
        let mut messages: Vec<TeamMessage> = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        messages.reverse();

        let has_more = match messages.first() {
            Some(first) => Self::has_team_messages_before(&conn, team_id, first)?,
            None => false,
        };

        Ok((messages, has_more))
    }

    fn row_to_team_message(row: &rusqlite::Row) -> rusqlite::Result<TeamMessage> {
        Ok(TeamMessage {
            message_id: row.get(0)?,
            team_id: row.get(1)?,
            from_member_id: row.get(2)?,
            to_member_id: row.get(3)?,
            content: row.get(4)?,
            message_type: TeamMessageType::from_str(&row.get::<_, String>(5)?),
            timestamp: row.get(6)?,
        })
    }

    fn has_team_messages_before(
        conn: &rusqlite::Connection,
        team_id: &str,
        first: &TeamMessage,
    ) -> Result<bool> {
        let result: rusqlite::Result<i64> = conn.query_row(
            "SELECT 1 FROM team_messages
             WHERE team_id = ?1
               AND (timestamp < ?2
                    OR (timestamp = ?2 AND message_id < ?3))
             LIMIT 1",
            params![team_id, first.timestamp, first.message_id],
            |row| row.get(0),
        );
        match result {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    // ── Team Tasks ──────────────────────────────────────────────

    pub fn insert_team_task(&self, task: &TeamTask) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let blocked_by = serde_json::to_string(&task.blocked_by)?;
        let blocks = serde_json::to_string(&task.blocks)?;
        conn.execute(
            "INSERT INTO team_tasks (team_id, content, status, owner_member_id, priority,
             blocked_by, blocks, column_name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                task.team_id,
                task.content,
                task.status,
                task.owner_member_id,
                task.priority,
                blocked_by,
                blocks,
                task.column_name,
                task.created_at,
                task.updated_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Model-plane task creation with actor authorization, owner resolution,
    /// task insert, and member assignment in one transaction.
    pub fn insert_authorized_team_task(
        &self,
        team_id: &str,
        session_id: &str,
        content: &str,
        owner: Option<&str>,
        priority: Option<u32>,
        blocked_by: Vec<i64>,
    ) -> Result<TeamTask> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        Self::authorize_active_team_actor(&tx, team_id, session_id)?;
        let owner_member_id = match owner {
            Some(owner) => Some(
                tx.query_row(
                    "SELECT member_id FROM team_members
                     WHERE team_id = ?1 AND (member_id = ?2 OR name = ?2)
                       AND status IN ('idle', 'working')",
                    params![team_id, owner],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        anyhow::anyhow!("Task owner is not an active member of this team")
                    }
                    other => other.into(),
                })?,
            ),
            None => None,
        };
        let now = chrono::Utc::now().to_rfc3339();
        let blocked_by_json = serde_json::to_string(&blocked_by)?;
        let column_name = if owner_member_id.is_some() {
            "doing"
        } else {
            "todo"
        };
        tx.execute(
            "INSERT INTO team_tasks (team_id, content, status, owner_member_id, priority,
             blocked_by, blocks, column_name, created_at, updated_at)
             VALUES (?1, ?2, 'pending', ?3, ?4, ?5, '[]', ?6, ?7, ?7)",
            params![
                team_id,
                content,
                owner_member_id,
                priority.unwrap_or(100),
                blocked_by_json,
                column_name,
                now,
            ],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(owner_member_id) = owner_member_id.as_deref() {
            tx.execute(
                "UPDATE team_members SET current_task_id = ?1
                 WHERE member_id = ?2 AND team_id = ?3",
                params![id, owner_member_id, team_id],
            )?;
        }
        tx.commit()?;
        Ok(TeamTask {
            id,
            team_id: team_id.to_string(),
            content: content.to_string(),
            status: "pending".to_string(),
            owner_member_id,
            priority: priority.unwrap_or(100),
            blocked_by,
            blocks: Vec::new(),
            column_name: column_name.to_string(),
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn get_team_task(&self, task_id: i64) -> Result<Option<TeamTask>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let result = conn.query_row(
            "SELECT id, team_id, content, status, owner_member_id, priority,
                    blocked_by, blocks, column_name, created_at, updated_at
             FROM team_tasks WHERE id = ?1",
            params![task_id],
            Self::row_to_team_task,
        );
        match result {
            Ok(t) => Ok(Some(t)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_team_tasks(&self, team_id: &str) -> Result<Vec<TeamTask>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, team_id, content, status, owner_member_id, priority,
                    blocked_by, blocks, column_name, created_at, updated_at
             FROM team_tasks WHERE team_id = ?1 ORDER BY priority ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![team_id], Self::row_to_team_task)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_team_task(
        &self,
        task_id: i64,
        status: Option<&str>,
        owner: Option<&str>,
        column: Option<&str>,
        content: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut updates = vec!["updated_at = datetime('now')".to_string()];
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(s) = status {
            updates.push(format!("status = ?{}", values.len() + 1));
            values.push(Box::new(s.to_string()));
        }
        if let Some(o) = owner {
            updates.push(format!("owner_member_id = ?{}", values.len() + 1));
            values.push(Box::new(o.to_string()));
        }
        if let Some(c) = column {
            updates.push(format!("column_name = ?{}", values.len() + 1));
            values.push(Box::new(c.to_string()));
        }
        if let Some(ct) = content {
            updates.push(format!("content = ?{}", values.len() + 1));
            values.push(Box::new(ct.to_string()));
        }

        let idx = values.len() + 1;
        let sql = format!(
            "UPDATE team_tasks SET {} WHERE id = ?{}",
            updates.join(", "),
            idx
        );
        values.push(Box::new(task_id));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|b| b.as_ref()).collect();
        conn.execute(&sql, param_refs.as_slice())?;
        Ok(())
    }

    /// Model-plane task update with actor authorization and team-id scoping in
    /// one transaction. The explicit team predicate closes the historical
    /// cross-team task-id mutation gap.
    #[allow(clippy::too_many_arguments)]
    pub fn update_authorized_team_task(
        &self,
        team_id: &str,
        session_id: &str,
        task_id: i64,
        status: Option<&str>,
        owner: Option<&str>,
        column: Option<&str>,
        content: Option<&str>,
    ) -> Result<TeamTask> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        Self::authorize_active_team_actor(&tx, team_id, session_id)?;
        let owner_member_id = match owner {
            Some(owner) => Some(
                tx.query_row(
                    "SELECT member_id FROM team_members
                     WHERE team_id = ?1 AND (member_id = ?2 OR name = ?2)
                       AND status IN ('idle', 'working')",
                    params![team_id, owner],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        anyhow::anyhow!("Task owner is not an active member of this team")
                    }
                    other => other.into(),
                })?,
            ),
            None => None,
        };
        let updated = tx.execute(
            "UPDATE team_tasks SET
                 status = COALESCE(?1, status),
                 owner_member_id = COALESCE(?2, owner_member_id),
                 column_name = COALESCE(?3, column_name),
                 content = COALESCE(?4, content),
                 updated_at = datetime('now')
             WHERE id = ?5 AND team_id = ?6",
            params![status, owner_member_id, column, content, task_id, team_id],
        )?;
        if updated != 1 {
            anyhow::bail!("Task not found in this team");
        }
        if let Some(owner_member_id) = owner_member_id.as_deref() {
            tx.execute(
                "UPDATE team_members SET current_task_id = ?1
                 WHERE member_id = ?2 AND team_id = ?3",
                params![task_id, owner_member_id, team_id],
            )?;
        }
        let task = tx.query_row(
            "SELECT id, team_id, content, status, owner_member_id, priority,
                    blocked_by, blocks, column_name, created_at, updated_at
             FROM team_tasks WHERE id = ?1 AND team_id = ?2",
            params![task_id, team_id],
            Self::row_to_team_task,
        )?;
        tx.commit()?;
        Ok(task)
    }

    pub fn delete_team_task(&self, task_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute("DELETE FROM team_tasks WHERE id = ?1", params![task_id])?;
        Ok(())
    }

    fn row_to_team_task(row: &rusqlite::Row) -> rusqlite::Result<TeamTask> {
        let blocked_by_str: String = row.get(6)?;
        let blocks_str: String = row.get(7)?;
        Ok(TeamTask {
            id: row.get(0)?,
            team_id: row.get(1)?,
            content: row.get(2)?,
            status: row.get(3)?,
            owner_member_id: row.get(4)?,
            priority: row.get::<_, i64>(5)? as u32,
            blocked_by: serde_json::from_str(&blocked_by_str).unwrap_or_default(),
            blocks: serde_json::from_str(&blocks_str).unwrap_or_default(),
            column_name: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }

    // ── Team Templates ──────────────────────────────────────────

    /// Insert or replace a team template. Returns the stored row with
    /// server-assigned `created_at` / `updated_at` so callers don't need a
    /// follow-up SELECT to read back the timestamps.
    pub fn insert_team_template(&self, tpl: &TeamTemplate) -> Result<TeamTemplate> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let members_json = serde_json::to_string(&tpl.members)?;
        let now = chrono::Utc::now().to_rfc3339();
        let created_at = if tpl.created_at.is_empty() {
            now.clone()
        } else {
            tpl.created_at.clone()
        };
        let updated_at = if tpl.updated_at.is_empty() {
            now
        } else {
            tpl.updated_at.clone()
        };
        conn.execute(
            "INSERT OR REPLACE INTO team_templates (template_id, name, description,
             members_json, builtin, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
            params![
                tpl.template_id,
                tpl.name,
                tpl.description,
                members_json,
                created_at,
                updated_at,
            ],
        )?;
        Ok(TeamTemplate {
            template_id: tpl.template_id.clone(),
            name: tpl.name.clone(),
            description: tpl.description.clone(),
            members: tpl.members.clone(),
            created_at,
            updated_at,
        })
    }

    pub fn list_team_templates(&self) -> Result<Vec<TeamTemplate>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT template_id, name, description, members_json, created_at, updated_at
             FROM team_templates ORDER BY updated_at DESC, name ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let members_json: String = row.get(3)?;
            Ok(TeamTemplate {
                template_id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                members: serde_json::from_str(&members_json).unwrap_or_default(),
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_team_template(&self, template_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM team_templates WHERE template_id = ?1",
            params![template_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagent::{SubagentDeliveryKind, SubagentOwnerKind, SubagentRun, SubagentStatus};

    fn test_db() -> (tempfile::TempDir, SessionDB) {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&temp.path().join("sessions.db"))
            .expect("open test db");
        (temp, db)
    }

    fn team(team_id: &str, lead_session_id: &str) -> Team {
        Team {
            team_id: team_id.to_string(),
            name: format!("Team {team_id}"),
            description: None,
            lead_session_id: lead_session_id.to_string(),
            lead_agent_id: "lead-agent".to_string(),
            status: TeamStatus::Active,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            template_id: None,
            config: TeamConfig::default(),
        }
    }

    fn member(team_id: &str, member_id: &str, run_id: &str, session_id: &str) -> TeamMember {
        TeamMember {
            member_id: member_id.to_string(),
            team_id: team_id.to_string(),
            name: member_id.to_string(),
            agent_id: "member-agent".to_string(),
            role: MemberRole::Worker,
            status: MemberStatus::Working,
            run_id: Some(run_id.to_string()),
            session_id: Some(session_id.to_string()),
            color: "#3B82F6".to_string(),
            current_task_id: None,
            model_override: None,
            role_description: None,
            joined_at: "2026-01-01T00:00:00Z".to_string(),
            last_active_at: None,
            input_tokens: Some(0),
            output_tokens: Some(0),
        }
    }

    fn idle_member(team_id: &str, member_id: &str) -> TeamMember {
        let mut member = member(team_id, member_id, "unused-run", "unused-session");
        member.status = MemberStatus::Idle;
        member.run_id = None;
        member.session_id = None;
        member
    }

    fn run(
        team_id: &str,
        lead_session_id: &str,
        run_id: &str,
        child_session_id: &str,
        status: SubagentStatus,
    ) -> SubagentRun {
        SubagentRun {
            run_id: run_id.to_string(),
            thread_id: child_session_id.to_string(),
            parent_session_id: lead_session_id.to_string(),
            parent_agent_id: "lead-agent".to_string(),
            child_agent_id: "member-agent".to_string(),
            child_session_id: child_session_id.to_string(),
            task: "work".to_string(),
            status,
            started_at: "2026-01-01T00:00:00Z".to_string(),
            delivery_kind: SubagentDeliveryKind::None,
            owner_kind: SubagentOwnerKind::Team,
            owner_id: team_id.to_string(),
            ..SubagentRun::default()
        }
    }

    fn seed_member(
        db: &SessionDB,
        team_id: &str,
        lead_session_id: &str,
        member_id: &str,
        run_id: &str,
        child_session_id: &str,
        status: SubagentStatus,
    ) {
        db.insert_subagent_run(&run(
            team_id,
            lead_session_id,
            run_id,
            child_session_id,
            status,
        ))
        .expect("insert subagent run");
        db.insert_team_member(&member(team_id, member_id, run_id, child_session_id))
            .expect("insert team member");
    }

    #[test]
    fn pause_resume_reuses_roster_rows_and_dissolved_is_terminal() {
        let (_temp, db) = test_db();
        db.insert_team(&team("team-1", "lead-1"))
            .expect("insert team");
        seed_member(
            &db,
            "team-1",
            "lead-1",
            "member-a",
            "run-a-old",
            "session-a-old",
            SubagentStatus::Running,
        );
        seed_member(
            &db,
            "team-1",
            "lead-1",
            "member-b",
            "run-b-old",
            "session-b-old",
            SubagentStatus::Queued,
        );
        seed_member(
            &db,
            "team-1",
            "lead-1",
            "member-completed",
            "run-completed",
            "session-completed",
            SubagentStatus::Completed,
        );

        let (paused_count, mut paused_runs) = db
            .pause_active_team_and_snapshot_runs("team-1")
            .expect("pause active team");
        paused_runs.sort();
        assert_eq!(paused_count, 2);
        assert_eq!(paused_runs, vec!["run-a-old", "run-b-old"]);
        assert_eq!(
            db.get_team("team-1").unwrap().unwrap().status,
            TeamStatus::Paused
        );
        let paused_members = db.list_team_members("team-1").unwrap();
        assert_eq!(
            paused_members
                .iter()
                .find(|member| member.member_id == "member-completed")
                .unwrap()
                .status,
            MemberStatus::Completed
        );
        assert!(paused_members
            .iter()
            .filter(|member| member.member_id != "member-completed")
            .all(|member| member.status == MemberStatus::Paused));
        assert!(db.pause_active_team_and_snapshot_runs("team-1").is_err());

        for run_id in ["run-a-old", "run-b-old"] {
            db.update_subagent_status(run_id, SubagentStatus::Killed, None, None, None, Some(1))
                .expect("settle pause cancellation");
        }
        let (_active_team, original_members, pending_members, completed_during_pause) =
            db.begin_resume_team("team-1").expect("claim resume");
        assert!(pending_members.is_empty());
        assert!(completed_during_pause.is_empty());
        let original_ids: std::collections::HashSet<_> = original_members
            .iter()
            .map(|member| member.member_id.clone())
            .collect();
        assert_eq!(original_ids.len(), 2);

        let rogue = run(
            "other-team",
            "lead-1",
            "run-rogue",
            "session-rogue",
            SubagentStatus::Running,
        );
        db.insert_subagent_run(&rogue).expect("insert rogue run");
        assert!(!db
            .activate_team_member_attempt(
                "team-1",
                "member-a",
                &MemberStatus::Paused,
                Some("run-a-old"),
                Some("session-a-old"),
                "run-rogue",
                "session-rogue",
            )
            .unwrap());

        for (member_id, run_id, session_id) in [
            ("member-a", "run-a-new", "session-a-new"),
            ("member-b", "run-b-new", "session-b-new"),
        ] {
            db.insert_subagent_run(&run(
                "team-1",
                "lead-1",
                run_id,
                session_id,
                SubagentStatus::Spawning,
            ))
            .expect("insert resumed run");
            assert!(db
                .activate_team_member_attempt(
                    "team-1",
                    member_id,
                    &MemberStatus::Paused,
                    Some(if member_id == "member-a" {
                        "run-a-old"
                    } else {
                        "run-b-old"
                    }),
                    Some(if member_id == "member-a" {
                        "session-a-old"
                    } else {
                        "session-b-old"
                    }),
                    run_id,
                    session_id,
                )
                .unwrap());
        }

        let resumed_members = db.list_team_members("team-1").unwrap();
        assert_eq!(resumed_members.len(), 3);
        assert_eq!(
            resumed_members
                .iter()
                .filter(|member| member.member_id != "member-completed")
                .map(|member| member.member_id.clone())
                .collect::<std::collections::HashSet<_>>(),
            original_ids
        );
        assert!(resumed_members
            .iter()
            .filter(|member| member.member_id != "member-completed")
            .all(|member| member.status == MemberStatus::Working));
        assert_eq!(
            resumed_members
                .iter()
                .find(|member| member.member_id == "member-completed")
                .unwrap()
                .status,
            MemberStatus::Completed
        );
        assert!(db.begin_resume_team("team-1").is_err());

        let (_team, mut dissolved_runs) = db
            .dissolve_team_and_snapshot_runs("team-1")
            .expect("dissolve active team");
        dissolved_runs.sort();
        assert_eq!(dissolved_runs, vec!["run-a-new", "run-b-new"]);
        assert_eq!(
            db.get_team("team-1").unwrap().unwrap().status,
            TeamStatus::Dissolved
        );
        let dissolved_members = db.list_team_members("team-1").unwrap();
        assert!(dissolved_members
            .iter()
            .filter(|member| member.member_id != "member-completed")
            .all(|member| member.status == MemberStatus::Killed));
        assert_eq!(
            dissolved_members
                .iter()
                .find(|member| member.member_id == "member-completed")
                .unwrap()
                .status,
            MemberStatus::Completed
        );
        assert!(db.begin_resume_team("team-1").is_err());
        assert!(db.pause_active_team_and_snapshot_runs("team-1").is_err());
        assert!(db.dissolve_team_and_snapshot_runs("team-1").is_err());
    }

    #[test]
    fn resume_refuses_the_whole_team_until_old_attempts_are_terminal() {
        let (_temp, db) = test_db();
        db.insert_team(&team("team-pending", "lead-pending"))
            .expect("insert team");
        seed_member(
            &db,
            "team-pending",
            "lead-pending",
            "member-pending",
            "run-pending",
            "session-pending",
            SubagentStatus::Running,
        );
        db.pause_active_team_and_snapshot_runs("team-pending")
            .expect("pause team");

        let (team, eligible, pending, completed) = db
            .begin_resume_team("team-pending")
            .expect("inspect blocked resume");
        assert_eq!(team.status, TeamStatus::Paused);
        assert!(eligible.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0.member_id, "member-pending");
        assert_eq!(pending[0].1, "old_attempt_still_active");
        assert_eq!(pending[0].2, "running");
        assert!(completed.is_empty());
        assert_eq!(
            db.get_team("team-pending").unwrap().unwrap().status,
            TeamStatus::Paused
        );
        assert_eq!(
            db.team_member_resume_blocker("team-pending", "member-pending", Some("run-pending"))
                .unwrap_err()
                .to_string(),
            "Team/member state changed before resume launch"
        );

        db.update_subagent_status(
            "run-pending",
            SubagentStatus::Killed,
            None,
            None,
            None,
            Some(1),
        )
        .expect("settle cancellation");
        let (team, eligible, pending, completed) = db
            .begin_resume_team("team-pending")
            .expect("claim settled resume");
        assert_eq!(team.status, TeamStatus::Active);
        assert_eq!(eligible.len(), 1);
        assert!(pending.is_empty());
        assert!(completed.is_empty());
    }

    #[test]
    fn resume_fails_closed_when_a_non_null_run_id_has_no_durable_record() {
        let (_temp, db) = test_db();
        db.insert_team(&team("team-missing", "lead-missing"))
            .expect("insert team");
        db.insert_team_member(&member(
            "team-missing",
            "member-missing",
            "run-missing",
            "session-missing",
        ))
        .expect("insert member with missing run record");
        db.pause_active_team_and_snapshot_runs("team-missing")
            .expect("pause team");

        let (team, eligible, pending, completed) = db
            .begin_resume_team("team-missing")
            .expect("inspect missing-run resume");
        assert_eq!(team.status, TeamStatus::Paused);
        assert!(eligible.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0.member_id, "member-missing");
        assert_eq!(pending[0].1, "old_attempt_unknown");
        assert_eq!(pending[0].2, "missing_run_record");
        assert!(completed.is_empty());
        assert_eq!(
            db.get_team("team-missing").unwrap().unwrap().status,
            TeamStatus::Paused
        );

        // Simulate a claimed Active team to exercise the final pre-spawn gate
        // independently from begin_resume_team's whole-team gate.
        db.update_team_status("team-missing", &TeamStatus::Active)
            .expect("simulate claimed team");
        assert_eq!(
            db.team_member_resume_blocker("team-missing", "member-missing", Some("run-missing"))
                .unwrap(),
            Some((
                "old_attempt_unknown".to_string(),
                "missing_run_record".to_string()
            ))
        );
    }

    #[test]
    fn resume_reconciles_success_that_won_the_pause_cancel_race() {
        let (_temp, db) = test_db();
        db.insert_team(&team("team-late-success", "lead-late-success"))
            .expect("insert team");
        seed_member(
            &db,
            "team-late-success",
            "lead-late-success",
            "member-completed-before-pause",
            "run-completed-before-pause",
            "session-completed-before-pause",
            SubagentStatus::Completed,
        );
        seed_member(
            &db,
            "team-late-success",
            "lead-late-success",
            "member-late-success",
            "run-late-success",
            "session-late-success",
            SubagentStatus::Running,
        );
        db.pause_active_team_and_snapshot_runs("team-late-success")
            .expect("pause team");

        // Cooperative cancellation lost to successful completion after the
        // roster row was already Paused.
        db.update_subagent_status(
            "run-late-success",
            SubagentStatus::Completed,
            Some("done"),
            None,
            None,
            Some(1),
        )
        .expect("late success");

        let (team, eligible, pending, completed) = db
            .begin_resume_team("team-late-success")
            .expect("resume inspection is a structured no-op");
        assert_eq!(team.status, TeamStatus::Paused);
        assert!(eligible.is_empty(), "successful work must not be repeated");
        assert!(pending.is_empty());
        assert_eq!(completed.len(), 2);
        assert!(completed
            .iter()
            .all(|member| member.status == MemberStatus::Completed));
        let first_completed_ids: Vec<_> = completed
            .iter()
            .map(|member| member.member_id.as_str())
            .collect();
        assert_eq!(
            first_completed_ids,
            vec!["member-completed-before-pause", "member-late-success"]
        );
        assert_eq!(
            db.get_team_member("member-late-success")
                .unwrap()
                .unwrap()
                .status,
            MemberStatus::Completed
        );
        assert_eq!(
            db.get_team("team-late-success").unwrap().unwrap().status,
            TeamStatus::Paused,
            "an already-complete resume is a no-op, not an implicit unpause"
        );

        // Refresh/retry after the first reconciliation must return the same
        // non-empty completed roster instead of falling into "no paused
        // members" or claiming the team Active.
        let run_count_before_retry = db
            .list_subagent_runs("lead-late-success")
            .expect("list runs before retry")
            .len();
        let (team, eligible, pending, completed_again) = db
            .begin_resume_team("team-late-success")
            .expect("repeated resume remains an already-complete no-op");
        assert_eq!(team.status, TeamStatus::Paused);
        assert!(eligible.is_empty());
        assert!(pending.is_empty());
        assert_eq!(
            completed_again
                .iter()
                .map(|member| member.member_id.as_str())
                .collect::<Vec<_>>(),
            first_completed_ids
        );
        assert_eq!(
            db.list_subagent_runs("lead-late-success")
                .expect("list runs after retry")
                .len(),
            run_count_before_retry,
            "an idempotent resume must not materialize another attempt"
        );
    }

    #[test]
    fn no_paused_member_is_not_already_complete_for_empty_or_mixed_rosters() {
        let (_temp, db) = test_db();

        db.insert_team(&team("team-empty", "lead-empty"))
            .expect("insert empty team");
        let (paused_count, paused_runs) = db
            .pause_active_team_and_snapshot_runs("team-empty")
            .expect("pause empty team");
        assert_eq!(paused_count, 0);
        assert!(paused_runs.is_empty());
        assert_eq!(
            db.begin_resume_team("team-empty")
                .expect_err("empty roster is not successful completion")
                .to_string(),
            "Team has no paused members to resume"
        );

        db.insert_team(&team("team-mixed", "lead-mixed"))
            .expect("insert mixed team");
        seed_member(
            &db,
            "team-mixed",
            "lead-mixed",
            "member-error",
            "run-error",
            "session-error",
            SubagentStatus::Error,
        );
        seed_member(
            &db,
            "team-mixed",
            "lead-mixed",
            "member-late-success",
            "run-mixed-late-success",
            "session-mixed-late-success",
            SubagentStatus::Running,
        );
        db.pause_active_team_and_snapshot_runs("team-mixed")
            .expect("pause mixed team");
        db.update_subagent_status(
            "run-mixed-late-success",
            SubagentStatus::Completed,
            Some("done"),
            None,
            None,
            Some(1),
        )
        .expect("late success in mixed roster");
        assert_eq!(
            db.begin_resume_team("team-mixed")
                .expect_err("Error + Completed roster is not all complete")
                .to_string(),
            "Team has no paused members to resume"
        );
        assert_eq!(
            db.get_team("team-mixed").unwrap().unwrap().status,
            TeamStatus::Paused
        );
    }

    #[test]
    fn prepared_attempt_launch_claim_loses_cleanly_to_pause_and_dissolve() {
        let (_temp, db) = test_db();

        // Attach first, then pause before launch. The pause snapshot must see
        // the exact prepared run and the final launch CAS must lose.
        db.insert_team(&team("team-pause-race", "lead-pause-race"))
            .expect("insert pause-race team");
        db.insert_team_member(&idle_member("team-pause-race", "member-pause-race"))
            .expect("insert idle member");
        db.insert_subagent_run(&run(
            "team-pause-race",
            "lead-pause-race",
            "run-already-running",
            "session-already-running",
            SubagentStatus::Running,
        ))
        .expect("insert already-running attempt");
        assert!(
            !db.activate_team_member_attempt(
                "team-pause-race",
                "member-pause-race",
                &MemberStatus::Idle,
                None,
                None,
                "run-already-running",
                "session-already-running",
            )
            .unwrap(),
            "an executor that is already Running can never be attached after the fact"
        );
        db.insert_subagent_run(&run(
            "team-pause-race",
            "lead-pause-race",
            "run-pause-prepared",
            "session-pause-prepared",
            SubagentStatus::Spawning,
        ))
        .expect("insert prepared pause run");
        assert!(db
            .activate_team_member_attempt(
                "team-pause-race",
                "member-pause-race",
                &MemberStatus::Idle,
                None,
                None,
                "run-pause-prepared",
                "session-pause-prepared",
            )
            .unwrap());
        let (_, pause_runs) = db
            .pause_active_team_and_snapshot_runs("team-pause-race")
            .expect("pause wins before launch");
        assert_eq!(pause_runs, vec!["run-pause-prepared"]);
        assert!(!db
            .claim_team_member_attempt_launch(
                "team-pause-race",
                "member-pause-race",
                "run-pause-prepared",
                "session-pause-prepared",
                &SubagentStatus::Spawning,
            )
            .unwrap());

        // Dissolve commits before attach. The prepared run is deliberately not
        // in the lifecycle snapshot, but attach and launch are both denied; the
        // caller-owned prepared handle is then responsible for terminal cleanup.
        db.insert_team(&team("team-dissolve-race", "lead-dissolve-race"))
            .expect("insert dissolve-race team");
        db.insert_team_member(&idle_member("team-dissolve-race", "member-dissolve-race"))
            .expect("insert idle member");
        db.insert_subagent_run(&run(
            "team-dissolve-race",
            "lead-dissolve-race",
            "run-dissolve-prepared",
            "session-dissolve-prepared",
            SubagentStatus::Spawning,
        ))
        .expect("insert prepared dissolve run");
        let (_, dissolve_runs) = db
            .dissolve_team_and_snapshot_runs("team-dissolve-race")
            .expect("dissolve wins before attach");
        assert!(dissolve_runs.is_empty());
        assert!(!db
            .activate_team_member_attempt(
                "team-dissolve-race",
                "member-dissolve-race",
                &MemberStatus::Idle,
                None,
                None,
                "run-dissolve-prepared",
                "session-dissolve-prepared",
            )
            .unwrap());
        assert!(!db
            .claim_team_member_attempt_launch(
                "team-dissolve-race",
                "member-dissolve-race",
                "run-dissolve-prepared",
                "session-dissolve-prepared",
                &SubagentStatus::Spawning,
            )
            .unwrap());
    }

    #[test]
    fn unlaunched_attempt_rollback_is_exact_and_does_not_override_pause() {
        let (_temp, db) = test_db();
        db.insert_team(&team("team-rollback", "lead-rollback"))
            .expect("insert team");
        db.insert_team_member(&idle_member("team-rollback", "member-rollback"))
            .expect("insert member");
        db.insert_subagent_run(&run(
            "team-rollback",
            "lead-rollback",
            "run-rollback",
            "session-rollback",
            SubagentStatus::Spawning,
        ))
        .expect("insert prepared run");
        assert!(db
            .activate_team_member_attempt(
                "team-rollback",
                "member-rollback",
                &MemberStatus::Idle,
                None,
                None,
                "run-rollback",
                "session-rollback",
            )
            .unwrap());
        assert!(db
            .restore_team_member_after_unlaunched_attempt(
                "team-rollback",
                "member-rollback",
                "run-rollback",
                "session-rollback",
                &MemberStatus::Idle,
                None,
                None,
            )
            .unwrap());
        let restored = db.get_team_member("member-rollback").unwrap().unwrap();
        assert_eq!(restored.status, MemberStatus::Idle);
        assert!(restored.run_id.is_none());

        assert!(db
            .activate_team_member_attempt(
                "team-rollback",
                "member-rollback",
                &MemberStatus::Idle,
                None,
                None,
                "run-rollback",
                "session-rollback",
            )
            .unwrap());
        db.pause_active_team_and_snapshot_runs("team-rollback")
            .expect("pause after attach");
        assert!(!db
            .restore_team_member_after_unlaunched_attempt(
                "team-rollback",
                "member-rollback",
                "run-rollback",
                "session-rollback",
                &MemberStatus::Idle,
                None,
                None,
            )
            .unwrap());
        assert_eq!(
            db.get_team_member("member-rollback")
                .unwrap()
                .unwrap()
                .status,
            MemberStatus::Paused
        );
    }

    #[test]
    fn model_collaboration_requires_active_live_member_and_scopes_task_ids() {
        let (_temp, db) = test_db();
        db.insert_team(&team("team-1", "lead-1"))
            .expect("insert team one");
        db.insert_team(&team("team-2", "lead-2"))
            .expect("insert team two");
        seed_member(
            &db,
            "team-1",
            "lead-1",
            "member-a",
            "run-a",
            "member-session-a",
            SubagentStatus::Running,
        );

        let member_task = db
            .insert_authorized_team_task(
                "team-1",
                "member-session-a",
                "member task",
                Some("member-a"),
                None,
                Vec::new(),
            )
            .expect("live member may collaborate");
        assert_eq!(member_task.team_id, "team-1");

        let foreign_task = db
            .insert_authorized_team_task("team-2", "lead-2", "foreign task", None, None, Vec::new())
            .expect("lead creates foreign task");
        assert!(db
            .update_authorized_team_task(
                "team-1",
                "lead-1",
                foreign_task.id,
                Some("completed"),
                None,
                None,
                None,
            )
            .is_err());
        assert_eq!(
            db.get_team_task(foreign_task.id).unwrap().unwrap().status,
            "pending"
        );

        db.update_subagent_status(
            "run-a",
            SubagentStatus::Completed,
            Some("done"),
            None,
            None,
            Some(1),
        )
        .expect("complete member run");
        assert!(db
            .insert_authorized_team_task(
                "team-1",
                "member-session-a",
                "stale authority",
                None,
                None,
                Vec::new(),
            )
            .is_err());

        db.pause_active_team_and_snapshot_runs("team-1")
            .expect("pause team");
        assert!(db
            .insert_authorized_team_task(
                "team-1",
                "lead-1",
                "lead write while paused",
                None,
                None,
                Vec::new(),
            )
            .is_err());
    }
}
