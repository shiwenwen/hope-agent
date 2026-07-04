use std::sync::Arc;

use serde_json::json;

use crate::cron::CronDB;
use crate::loop_control::{
    CreateLoopScheduleInput, LoopExecutionStrategy, LoopSchedule, LoopTriggerKind,
};
use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};

fn display_only(content: String) -> CommandResult {
    CommandResult {
        content,
        action: Some(CommandAction::DisplayOnly),
    }
}

pub fn handle_loop(
    session_db: &Arc<SessionDB>,
    cron_db: &Arc<CronDB>,
    sid: &str,
    args: &str,
) -> Result<CommandResult, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() || matches!(first_word(trimmed), "status" | "list" | "show") {
        let rest = trimmed
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim())
            .unwrap_or("");
        return render_loop_status(session_db, sid, rest);
    }
    match first_word(trimmed) {
        "every" => create_every_loop(session_db, cron_db, sid, trimmed["every".len()..].trim()),
        "until" => create_until_loop(session_db, cron_db, sid, trimmed["until".len()..].trim()),
        "pause" => transition_loop(session_db, cron_db, sid, trimmed, LoopCommand::Pause),
        "resume" => transition_loop(session_db, cron_db, sid, trimmed, LoopCommand::Resume),
        "stop" | "cancel" => transition_loop(session_db, cron_db, sid, trimmed, LoopCommand::Stop),
        "help" => Ok(display_only(loop_usage())),
        _ => Err(loop_usage()),
    }
}

fn create_every_loop(
    session_db: &Arc<SessionDB>,
    cron_db: &Arc<CronDB>,
    sid: &str,
    raw: &str,
) -> Result<CommandResult, String> {
    let (head, prompt) = split_head_prompt(raw);
    let mut parts = head.split_whitespace();
    let interval = parts.next().ok_or_else(loop_usage)?;
    let interval_secs = parse_duration_secs(interval)
        .ok_or_else(|| "Usage: /loop every <duration>: <prompt>".to_string())?;
    let opts = parse_loop_options(parts.collect::<Vec<_>>().join(" ").as_str())?;
    let schedule = session_db
        .create_loop_schedule(
            cron_db,
            CreateLoopScheduleInput {
                session_id: sid.to_string(),
                goal_id: None,
                prompt: prompt.to_string(),
                trigger_kind: LoopTriggerKind::Interval,
                trigger_spec: json!({ "intervalSecs": interval_secs }),
                execution_strategy: opts.execution_strategy,
                max_runs: opts.max_runs,
                max_runtime_secs: opts.max_runtime_secs,
                token_budget: opts.token_budget,
                cost_budget_micros: opts.cost_budget_micros,
                agent_id: None,
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(display_only(render_loop_created(&schedule)))
}

fn create_until_loop(
    session_db: &Arc<SessionDB>,
    cron_db: &Arc<CronDB>,
    sid: &str,
    raw: &str,
) -> Result<CommandResult, String> {
    let (condition_part, prompt) = split_head_prompt(raw);
    let (condition, interval_secs, opts) = parse_until_head(condition_part)?;
    if opts.execution_strategy == LoopExecutionStrategy::Workflow {
        return Err(
            "`/loop until` does not support `--workflow` yet; use `/loop every ... --workflow`."
                .to_string(),
        );
    }
    let recurring_prompt = if prompt.trim().is_empty() {
        format!(
            "Continue until this condition is true: {}. Check the condition first, stop when it is satisfied, otherwise take the next useful step.",
            condition
        )
    } else {
        prompt.to_string()
    };
    let schedule = session_db
        .create_loop_schedule(
            cron_db,
            CreateLoopScheduleInput {
                session_id: sid.to_string(),
                goal_id: None,
                prompt: recurring_prompt,
                trigger_kind: LoopTriggerKind::Condition,
                trigger_spec: json!({
                    "condition": condition,
                    "intervalSecs": interval_secs,
                }),
                execution_strategy: LoopExecutionStrategy::Continue,
                max_runs: opts.max_runs,
                max_runtime_secs: opts.max_runtime_secs,
                token_budget: opts.token_budget,
                cost_budget_micros: opts.cost_budget_micros,
                agent_id: None,
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(display_only(render_loop_created(&schedule)))
}

fn render_loop_status(
    session_db: &Arc<SessionDB>,
    sid: &str,
    maybe_id: &str,
) -> Result<CommandResult, String> {
    if !maybe_id.trim().is_empty() {
        let schedule = resolve_loop_schedule(session_db, sid, maybe_id.trim())?;
        let snapshot = session_db
            .loop_snapshot(&schedule.id, 10)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Loop not found".to_string())?;
        return Ok(display_only(render_loop_snapshot(&snapshot)));
    }
    let schedules = session_db
        .list_loop_schedules_for_session(sid, 20)
        .map_err(|e| e.to_string())?;
    if schedules.is_empty() {
        return Ok(display_only(
            "No loop schedules for this session.\n\nUse `/loop every 10m: <prompt>` or `/loop until <condition>`."
                .to_string(),
        ));
    }
    let mut lines = vec![format!("## Loops ({})", schedules.len())];
    for schedule in schedules {
        lines.push(format!(
            "- `{}` · **{}** · {} · {} · runs {}/{} · {}",
            short_id(&schedule.id),
            schedule.state.as_str(),
            schedule.execution_strategy.as_str(),
            trigger_summary(&schedule),
            schedule.run_count,
            schedule
                .max_runs
                .map(|v| v.to_string())
                .unwrap_or_else(|| "∞".to_string()),
            truncate(&schedule.prompt, 96)
        ));
    }
    lines.push(
        "\nUse `/loop status <id>` for trace, `/loop pause|resume|stop <id>` to control it.".into(),
    );
    Ok(display_only(lines.join("\n")))
}

enum LoopCommand {
    Pause,
    Resume,
    Stop,
}

fn transition_loop(
    session_db: &Arc<SessionDB>,
    cron_db: &Arc<CronDB>,
    sid: &str,
    raw: &str,
    command: LoopCommand,
) -> Result<CommandResult, String> {
    let id = raw
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "Pass a loop id or short id prefix.".to_string())?;
    let schedule = resolve_loop_schedule(session_db, sid, id)?;
    let next = match command {
        LoopCommand::Pause => session_db.pause_loop_schedule(cron_db, &schedule.id),
        LoopCommand::Resume => session_db.resume_loop_schedule(cron_db, &schedule.id),
        LoopCommand::Stop => session_db.stop_loop_schedule(cron_db, &schedule.id),
    }
    .map_err(|e| e.to_string())?;
    Ok(display_only(render_loop_created(&next)))
}

fn resolve_loop_schedule(
    session_db: &Arc<SessionDB>,
    sid: &str,
    id_or_prefix: &str,
) -> Result<LoopSchedule, String> {
    let schedules = session_db
        .list_loop_schedules_for_session(sid, 200)
        .map_err(|e| e.to_string())?;
    let matches: Vec<LoopSchedule> = schedules
        .into_iter()
        .filter(|s| s.id == id_or_prefix || s.id.starts_with(id_or_prefix))
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err(format!(
            "Loop '{}' not found for this session.",
            id_or_prefix
        )),
        _ => Err(format!(
            "Multiple loops match '{}'; pass a longer id.",
            id_or_prefix
        )),
    }
}

fn render_loop_created(schedule: &LoopSchedule) -> String {
    format!(
        "## Loop `{}`\n\nState: **{}** · Strategy: **{}** · {} · runs {}/{}\n\nPrompt:\n{}",
        short_id(&schedule.id),
        schedule.state.as_str(),
        schedule.execution_strategy.as_str(),
        trigger_summary(schedule),
        schedule.run_count,
        schedule
            .max_runs
            .map(|v| v.to_string())
            .unwrap_or_else(|| "∞".to_string()),
        schedule.prompt
    )
}

fn render_loop_snapshot(snapshot: &crate::loop_control::LoopSnapshot) -> String {
    let mut lines = vec![render_loop_created(&snapshot.schedule)];
    if let Some(reason) = snapshot.schedule.blocked_reason.as_deref() {
        lines.push(format!("\nBlocked: {}", reason));
    }
    if snapshot.runs.is_empty() {
        lines.push("\nNo runs yet.".into());
    } else {
        lines.push("\nRecent runs:".into());
        for run in &snapshot.runs {
            let mut details = Vec::new();
            if let Some(workflow_run_id) = run
                .trace
                .get("workflowRunId")
                .and_then(|value| value.as_str())
            {
                details.push(format!("workflow `{}`", short_id(workflow_run_id)));
            }
            if let Some(template_id) = run.trace.get("templateId").and_then(|value| value.as_str())
            {
                let template_version = run
                    .trace
                    .get("templateVersion")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let template = if template_version.is_empty() {
                    template_id.to_string()
                } else {
                    format!("{template_id}@{template_version}")
                };
                details.push(format!("template `{template}`"));
            }
            if let Some(summary) = run.result_summary.as_deref() {
                details.push(truncate(summary, 96).to_string());
            }
            lines.push(format!(
                "- #{} `{}` · {}{}{}",
                run.seq,
                run.state.as_str(),
                run.finished_at
                    .as_deref()
                    .unwrap_or(run.started_at.as_str()),
                if details.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", details.join(" · "))
                },
                run.error
                    .as_deref()
                    .map(|e| format!(" · {}", e))
                    .unwrap_or_default()
            ));
        }
    }
    lines.join("\n")
}

fn trigger_summary(schedule: &LoopSchedule) -> String {
    match schedule.trigger_kind {
        LoopTriggerKind::Interval => schedule
            .trigger_spec
            .get("intervalSecs")
            .and_then(|v| v.as_i64())
            .map(|secs| format!("every {}", format_duration(secs)))
            .unwrap_or_else(|| "interval".to_string()),
        LoopTriggerKind::Condition => {
            let condition = schedule
                .trigger_spec
                .get("condition")
                .and_then(|v| v.as_str())
                .unwrap_or("condition");
            format!("until {}", truncate(condition, 64))
        }
        LoopTriggerKind::Cron => "cron".into(),
        LoopTriggerKind::Event => "event".into(),
    }
}

#[derive(Default)]
struct LoopOptions {
    max_runs: Option<i64>,
    max_runtime_secs: Option<i64>,
    token_budget: Option<i64>,
    cost_budget_micros: Option<i64>,
    execution_strategy: LoopExecutionStrategy,
}

fn parse_loop_options(raw: &str) -> Result<LoopOptions, String> {
    let mut opts = LoopOptions::default();
    let mut iter = raw.split_whitespace();
    while let Some(flag) = iter.next() {
        match flag {
            "--max-runs" => {
                opts.max_runs = iter
                    .next()
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|v| *v > 0);
            }
            "--max-runtime" => {
                opts.max_runtime_secs = iter.next().and_then(parse_duration_secs);
            }
            "--tokens" | "--token-budget" => {
                opts.token_budget = iter
                    .next()
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|v| *v > 0);
            }
            "--cost-micros" => {
                opts.cost_budget_micros = iter
                    .next()
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|v| *v > 0);
            }
            "--workflow" => {
                opts.execution_strategy = LoopExecutionStrategy::Workflow;
            }
            "--continue" => {
                opts.execution_strategy = LoopExecutionStrategy::Continue;
            }
            "--strategy" | "--execution-strategy" => {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("{} requires continue or workflow", flag))?;
                opts.execution_strategy =
                    LoopExecutionStrategy::from_str(value).ok_or_else(|| {
                        format!(
                            "Invalid loop execution strategy `{value}`; use continue or workflow"
                        )
                    })?;
            }
            "" => {}
            other => return Err(format!("Unknown loop option `{}`", other)),
        }
    }
    Ok(opts)
}

fn parse_until_head(raw: &str) -> Result<(String, i64, LoopOptions), String> {
    let mut tokens = raw.split_whitespace().peekable();
    let mut condition = Vec::new();
    let mut interval_secs = 300;
    while let Some(token) = tokens.peek().copied() {
        if token == "every" {
            tokens.next();
            let value = tokens
                .next()
                .and_then(parse_duration_secs)
                .ok_or_else(|| "Usage: /loop until <condition> every <duration>".to_string())?;
            interval_secs = value;
            break;
        }
        if token.starts_with("--") {
            break;
        }
        condition.push(tokens.next().unwrap());
    }
    let opts = parse_loop_options(tokens.collect::<Vec<_>>().join(" ").as_str())?;
    let condition = condition.join(" ");
    if condition.trim().is_empty() {
        return Err("Usage: /loop until <condition> [every <duration>]: [prompt]".into());
    }
    Ok((condition, interval_secs, opts))
}

fn split_head_prompt(raw: &str) -> (&str, &str) {
    raw.split_once(':')
        .map(|(head, prompt)| (head.trim(), prompt.trim()))
        .unwrap_or((raw.trim(), ""))
}

fn first_word(input: &str) -> &str {
    input.split_whitespace().next().unwrap_or("")
}

fn parse_duration_secs(input: &str) -> Option<i64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let split = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (num, unit) = trimmed.split_at(split);
    let n = num.parse::<i64>().ok()?;
    let multiplier = match unit {
        "" | "s" | "sec" | "secs" => 1,
        "m" | "min" | "mins" => 60,
        "h" | "hr" | "hrs" => 3600,
        "d" | "day" | "days" => 86_400,
        _ => return None,
    };
    Some(n.saturating_mul(multiplier))
}

fn format_duration(secs: i64) -> String {
    if secs % 86_400 == 0 {
        format!("{}d", secs / 86_400)
    } else if secs % 3600 == 0 {
        format!("{}h", secs / 3600)
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn truncate(input: &str, max: usize) -> &str {
    if input.len() <= max {
        return input;
    }
    let mut end = max;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

fn loop_usage() -> String {
    [
        "Usage:",
        "- `/loop every 10m: <prompt>`: repeat a prompt on an interval",
        "- `/loop until <condition> [every 5m]: [prompt]`: poll until a condition is true",
        "- `/loop status [id]`: show loop schedules or a trace",
        "- `/loop pause|resume|stop <id>`: control a loop",
        "",
        "Options: `--max-runs N`, `--max-runtime 2h`, `--tokens N`, `--workflow`, `--strategy workflow|continue`.",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::CreateGoalInput;

    fn temp_dbs() -> (tempfile::TempDir, Arc<SessionDB>, Arc<CronDB>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let session_db =
            Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("session db"));
        {
            let conn = session_db.conn.lock().expect("lock session db");
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    chat_id TEXT NOT NULL,
                    thread_id TEXT,
                    session_id TEXT NOT NULL,
                    sender_id TEXT,
                    sender_name TEXT,
                    chat_type TEXT NOT NULL DEFAULT 'dm',
                    source TEXT NOT NULL DEFAULT 'inbound',
                    attached_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );",
            )
            .expect("channel conversations table");
        }
        let cron_db = Arc::new(CronDB::open(&dir.path().join("cron.db")).expect("cron db"));
        (dir, session_db, cron_db)
    }

    #[test]
    fn slash_every_workflow_creates_workflow_strategy_loop() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        session_db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Refresh the research brief".to_string(),
                completion_criteria: "Latest evidence is reviewed".to_string(),
                domain: Some("research".to_string()),
                workflow_template_id: Some("research-brief".to_string()),
                workflow_template_version: None,
                workflow_task_type: Some("technical_research".to_string()),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let result = handle_loop(
            &session_db,
            &cron_db,
            &session.id,
            "every 10m --workflow: Refresh sources",
        )
        .expect("create workflow loop");

        assert!(result.content.contains("Strategy: **workflow**"));
        let schedules = session_db
            .list_loop_schedules_for_session(&session.id, 10)
            .expect("list loops");
        assert_eq!(schedules.len(), 1);
        assert_eq!(
            schedules[0].execution_strategy,
            LoopExecutionStrategy::Workflow
        );
    }

    #[test]
    fn slash_loop_status_surfaces_workflow_run_trace() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let goal = session_db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Refresh the research brief".to_string(),
                completion_criteria: "Latest evidence is reviewed".to_string(),
                domain: Some("research".to_string()),
                workflow_template_id: Some("research-brief".to_string()),
                workflow_template_version: None,
                workflow_task_type: Some("technical_research".to_string()),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");
        let schedule = session_db
            .create_loop_schedule(
                &cron_db,
                CreateLoopScheduleInput {
                    session_id: session.id.clone(),
                    goal_id: Some(goal.goal.id),
                    prompt: "Refresh sources".to_string(),
                    trigger_kind: LoopTriggerKind::Interval,
                    trigger_spec: json!({ "intervalSecs": 600 }),
                    execution_strategy: LoopExecutionStrategy::Workflow,
                    max_runs: None,
                    max_runtime_secs: None,
                    token_budget: None,
                    cost_budget_micros: None,
                    agent_id: None,
                },
            )
            .expect("create loop");
        let started_at = chrono::Utc::now().to_rfc3339();
        let decision = session_db
            .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &started_at)
            .expect("prepare loop");
        let run_id = match decision {
            crate::loop_control::LoopRunDecision::Admit(admission) => admission.run_id,
            other => panic!("expected admit, got {other:?}"),
        };
        let workflow_run_id = "wfr_loop_generated_1234567890";
        session_db
            .finish_loop_cron_run_with_trace(
                &schedule.cron_job_id,
                Some(&run_id),
                None,
                crate::loop_control::LoopRunState::Succeeded,
                Some("workflow launched"),
                None,
                &chrono::Utc::now().to_rfc3339(),
                Some(json!({
                    "executionStrategy": "workflow",
                    "workflowRunId": workflow_run_id,
                    "templateId": "research-brief",
                    "templateVersion": "1.0.0",
                })),
            )
            .expect("finish loop");

        let status = handle_loop(
            &session_db,
            &cron_db,
            &session.id,
            &format!("status {}", schedule.id),
        )
        .expect("loop status");
        assert!(status.content.contains("Strategy: **workflow**"));
        assert!(status.content.contains("workflow `wfr_loop`"));
        assert!(status.content.contains("template `research-brief@1.0.0`"));
        assert!(status.content.contains("workflow launched"));
    }

    #[test]
    fn slash_until_rejects_workflow_strategy_until_condition_workflows_exist() {
        let (_dir, session_db, cron_db) = temp_dbs();
        let session = session_db.create_session("ha-main").expect("session");
        let err = handle_loop(
            &session_db,
            &cron_db,
            &session.id,
            "until CI is green every 5m --workflow: keep fixing",
        )
        .expect_err("condition workflow should be rejected");
        assert!(err.contains("does not support `--workflow`"));
    }
}
