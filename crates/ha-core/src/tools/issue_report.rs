use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::issue_reporting::{self, IssueDraft, IssueKind};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueReportArgs {
    action: String,
    #[serde(default)]
    kind: Option<IssueKind>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    #[serde(default)]
    duplicate_issue_urls: Vec<String>,
}

pub(crate) async fn tool_issue_report(
    args: &Value,
    ctx: &super::ToolExecContext,
) -> Result<String> {
    let parsed: IssueReportArgs = serde_json::from_value(args.clone())?;
    match parsed.action.as_str() {
        "search" => search(parsed).await,
        "draft" => draft(parsed),
        "create" => create(parsed, ctx).await,
        other => bail!("Unknown issue_report action: {other}. Expected search, draft, or create."),
    }
}

async fn search(args: IssueReportArgs) -> Result<String> {
    let cfg = crate::config::cached_config().issue_reporting.clone();
    let query = args
        .query
        .or(args.title)
        .ok_or_else(|| anyhow::anyhow!("search requires `query` or `title`"))?;
    let items = issue_reporting::search_issues(&cfg, &query).await?;
    Ok(serde_json::to_string_pretty(&json!({
        "action": "search",
        "owner": cfg.owner,
        "repo": cfg.repo,
        "query": query,
        "issues": items,
    }))?)
}

fn draft(args: IssueReportArgs) -> Result<String> {
    let cfg = crate::config::cached_config().issue_reporting.clone();
    let draft = build_draft(&cfg, args)?;
    Ok(serde_json::to_string_pretty(&json!({
        "action": "draft",
        "draft": draft,
        "note": "Review this draft with the user before calling issue_report(action=\"create\").",
    }))?)
}

async fn create(args: IssueReportArgs, ctx: &super::ToolExecContext) -> Result<String> {
    let cfg = crate::config::cached_config().issue_reporting.clone();
    let draft = build_draft(&cfg, args)?;
    confirm_create(&draft, ctx).await?;
    let created = issue_reporting::create_issue(&cfg, &draft).await?;
    Ok(serde_json::to_string_pretty(&json!({
        "action": "create",
        "created": created,
    }))?)
}

fn build_draft(
    cfg: &issue_reporting::IssueReportingConfig,
    args: IssueReportArgs,
) -> Result<IssueDraft> {
    let kind = args.kind.unwrap_or(IssueKind::Bug);
    let title = args
        .title
        .ok_or_else(|| anyhow::anyhow!("draft/create requires `title`"))?;
    let body = args
        .body
        .ok_or_else(|| anyhow::anyhow!("draft/create requires `body`"))?;

    let mut full_body = body;
    if !args.duplicate_issue_urls.is_empty() {
        full_body.push_str("\n\n## Possible duplicates checked\n");
        for url in &args.duplicate_issue_urls {
            full_body.push_str("- ");
            full_body.push_str(url);
            full_body.push('\n');
        }
    }
    if let Some(evidence) = args.evidence.filter(|s| !s.trim().is_empty()) {
        full_body.push_str("\n\n## Diagnostic evidence\n");
        full_body.push_str(&evidence);
    }

    issue_reporting::normalize_draft(cfg, kind, &title, &full_body, args.labels)
}

async fn confirm_create(draft: &IssueDraft, ctx: &super::ToolExecContext) -> Result<()> {
    let preview = format!(
        "Repository: {}/{}\nKind: {}\nTitle: {}\nLabels: {}\n\n{}",
        draft.owner,
        draft.repo,
        draft.kind.as_str(),
        draft.title,
        if draft.labels.is_empty() {
            "(none)".to_string()
        } else {
            draft.labels.join(", ")
        },
        crate::truncate_utf8(&draft.body, 4_000)
    );
    let response = super::ask_user_question::execute(
        &json!({
            "context": "Hope Agent is ready to create a GitHub issue. Confirm before anything is submitted.",
            "questions": [{
                "question_id": "confirm_create_issue",
                "header": "Create issue?",
                "text": "Create this GitHub issue now?",
                "options": [
                    {
                        "value": "create",
                        "label": "Create issue",
                        "description": "Submit the issue to GitHub.",
                        "recommended": true,
                        "preview": preview,
                        "previewKind": "markdown"
                    },
                    {
                        "value": "cancel",
                        "label": "Cancel",
                        "description": "Do not submit anything."
                    }
                ],
                "default_values": ["cancel"]
            }]
        }),
        ctx.session_id.as_deref(),
    )
    .await;

    if response.starts_with("Error:") {
        bail!(response);
    }
    if selected_create_issue(&response) {
        Ok(())
    } else {
        bail!("User cancelled GitHub issue creation");
    }
}

fn selected_create_issue(response: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(response) else {
        return false;
    };
    value
        .get("answers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|answer| {
            answer
                .get("selected")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .any(|label| label.as_str() == Some("Create issue"))
}
