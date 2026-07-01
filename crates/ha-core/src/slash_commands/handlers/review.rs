use std::sync::Arc;

use crate::review::{
    ReviewFinding, ReviewFindingStatus, ReviewRun, ReviewRunSnapshot, ReviewSeverity,
    ReviewVerdict, RunReviewInput,
};
use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};

fn display_only(content: String) -> CommandResult {
    CommandResult {
        content,
        action: Some(CommandAction::DisplayOnly),
    }
}

pub async fn handle_review(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or_else(|| "No active session for /review".to_string())?;
    let trimmed = args.trim();
    if trimmed.is_empty() || first_word(trimmed) == "run" {
        let snapshot = crate::review::run_review_for_session(
            session_db.clone(),
            sid.to_string(),
            RunReviewInput {
                scope: Some("local".to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        return Ok(display_only(render_review_snapshot(&snapshot)));
    }
    match first_word(trimmed) {
        "status" | "show" | "list" => render_review_status(session_db, sid, rest(trimmed)),
        "resolved" | "resolve" | "fixed" => update_finding(
            session_db,
            sid,
            rest(trimmed),
            ReviewFindingStatus::Resolved,
        ),
        "dismiss" | "dismissed" => update_finding(
            session_db,
            sid,
            rest(trimmed),
            ReviewFindingStatus::Dismissed,
        ),
        "false_positive" | "false-positive" | "fp" => update_finding(
            session_db,
            sid,
            rest(trimmed),
            ReviewFindingStatus::FalsePositive,
        ),
        "open" | "reopen" => {
            update_finding(session_db, sid, rest(trimmed), ReviewFindingStatus::Open)
        }
        "help" => Ok(display_only(review_usage())),
        _ => Err(review_usage()),
    }
}

fn render_review_status(
    session_db: &Arc<SessionDB>,
    sid: &str,
    maybe_id: &str,
) -> Result<CommandResult, String> {
    if !maybe_id.trim().is_empty() {
        let run = resolve_review_run(session_db, sid, maybe_id.trim())?;
        let snapshot = session_db
            .review_run_snapshot(&run.id, 100)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Review run not found".to_string())?;
        return Ok(display_only(render_review_snapshot(&snapshot)));
    }
    let runs = session_db
        .list_review_runs_for_session(sid, 20)
        .map_err(|e| e.to_string())?;
    if runs.is_empty() {
        return Ok(display_only(
            "No review runs for this session.\n\nUse `/review` to inspect local uncommitted changes."
                .to_string(),
        ));
    }
    let mut lines = vec![format!("## Review runs ({})", runs.len())];
    for run in runs {
        lines.push(format!(
            "- `{}` · **{}** · {} · {}",
            short_id(&run.id),
            run.state.as_str(),
            run.scope,
            truncate(&run.summary, 120)
        ));
    }
    lines.push("\nUse `/review status <id>` to inspect findings.".to_string());
    Ok(display_only(lines.join("\n")))
}

fn update_finding(
    session_db: &Arc<SessionDB>,
    sid: &str,
    id_or_prefix: &str,
    status: ReviewFindingStatus,
) -> Result<CommandResult, String> {
    let finding = resolve_review_finding(session_db, sid, id_or_prefix.trim())?;
    let updated = session_db
        .update_review_finding_status(&finding.id, status)
        .map_err(|e| e.to_string())?;
    Ok(display_only(format!(
        "Review finding `{}` is now **{}**.\n\n{}",
        short_id(&updated.id),
        updated.status.as_str(),
        render_finding_line(&updated)
    )))
}

fn resolve_review_run(
    session_db: &Arc<SessionDB>,
    sid: &str,
    id_or_prefix: &str,
) -> Result<ReviewRun, String> {
    let runs = session_db
        .list_review_runs_for_session(sid, 200)
        .map_err(|e| e.to_string())?;
    let matches: Vec<ReviewRun> = runs
        .into_iter()
        .filter(|run| run.id == id_or_prefix || run.id.starts_with(id_or_prefix))
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err(format!(
            "Review run '{}' not found for this session.",
            id_or_prefix
        )),
        _ => Err(format!(
            "Multiple review runs match '{}'; pass a longer id.",
            id_or_prefix
        )),
    }
}

fn resolve_review_finding(
    session_db: &Arc<SessionDB>,
    sid: &str,
    id_or_prefix: &str,
) -> Result<ReviewFinding, String> {
    if id_or_prefix.is_empty() {
        return Err("Pass a review finding id or short id prefix.".to_string());
    }
    let runs = session_db
        .list_review_runs_for_session(sid, 200)
        .map_err(|e| e.to_string())?;
    let mut matches = Vec::new();
    for run in runs {
        let findings = session_db
            .list_review_findings_for_run(&run.id)
            .map_err(|e| e.to_string())?;
        for finding in findings {
            if finding.id == id_or_prefix || finding.id.starts_with(id_or_prefix) {
                matches.push(finding);
            }
        }
    }
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err(format!(
            "Review finding '{}' not found for this session.",
            id_or_prefix
        )),
        _ => Err(format!(
            "Multiple review findings match '{}'; pass a longer id.",
            id_or_prefix
        )),
    }
}

fn render_review_snapshot(snapshot: &ReviewRunSnapshot) -> String {
    let mut lines = vec![
        format!("## Review `{}`", short_id(&snapshot.run.id)),
        String::new(),
        format!(
            "State: **{}** · Scope: `{}` · Findings: **{}**",
            snapshot.run.state.as_str(),
            snapshot.run.scope,
            snapshot.findings.len()
        ),
        String::new(),
        snapshot.run.summary.clone(),
    ];
    let open: Vec<&ReviewFinding> = snapshot
        .findings
        .iter()
        .filter(|finding| finding.status == ReviewFindingStatus::Open)
        .collect();
    if open.is_empty() {
        lines.push("\nNo open review findings.".to_string());
    } else {
        lines.push("\n### Open findings".to_string());
        for finding in open.into_iter().take(12) {
            lines.push(format!("- {}", render_finding_line(finding)));
        }
    }
    lines.join("\n")
}

fn render_finding_line(finding: &ReviewFinding) -> String {
    let loc = match (finding.start_line, finding.end_line) {
        (Some(start), Some(end)) if end > start => format!("{}:{}-{}", finding.file, start, end),
        (Some(start), _) => format!("{}:{}", finding.file, start),
        _ => finding.file.clone(),
    };
    format!(
        "`{}` · **{}** · {} · {} · {} — {}",
        short_id(&finding.id),
        severity_label(finding.severity),
        verdict_label(finding.verdict),
        finding.category,
        loc,
        finding.title
    )
}

fn review_usage() -> String {
    "Usage:\n- `/review` run local review for uncommitted changes\n- `/review status [id]` show recent review runs/findings\n- `/review resolved <finding>` mark fixed\n- `/review dismissed <finding>` ignore\n- `/review false_positive <finding>` mark as false positive\n- `/review open <finding>` reopen a finding".to_string()
}

fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

fn rest(s: &str) -> &str {
    s.split_once(char::is_whitespace)
        .map(|(_, rest)| rest.trim())
        .unwrap_or("")
}

fn short_id(id: &str) -> String {
    id.chars().take(10).collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

fn severity_label(severity: ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::P0 => "P0",
        ReviewSeverity::P1 => "P1",
        ReviewSeverity::P2 => "P2",
        ReviewSeverity::P3 => "P3",
    }
}

fn verdict_label(verdict: ReviewVerdict) -> &'static str {
    match verdict {
        ReviewVerdict::Confirmed => "confirmed",
        ReviewVerdict::Plausible => "plausible",
        ReviewVerdict::Refuted => "refuted",
    }
}
