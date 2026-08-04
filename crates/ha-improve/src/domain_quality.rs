//! 领域质量复核的**机器**（阶段 5 第八刀，自 ha-core 迁出）：把工作流证据、
//! 审批闸与画像规则判成一次确定性的质量复核。
//!
//! 台账（`domain_quality_*` 三张表的读写方法、wire 类型、行映射）留在
//! [`ha_core::domain_quality`]——本模块只经类型化仓储方法触达 `sessions.db`。

use anyhow::{anyhow, bail, Result};
use ha_core::domain_workflow::{
    DomainEvidenceItem, DomainEvidenceRequirement, DomainWorkflowTemplate, ListDomainEvidenceInput,
    ListDomainWorkflowTemplatesInput,
};
use ha_core::session::SessionDB;
use serde_json::{json, Value};
use std::collections::BTreeMap;

// 同名 kernel 模块留守的契约类型与类型化仓储方法。
use ha_core::domain_quality::*;

const DOMAIN_QUALITY_LIMIT_DEFAULT: usize = 50;

pub fn run_domain_quality_for_session(
    db: &SessionDB,
    input: RunDomainQualityInput,
) -> Result<DomainQualityRunSnapshot> {
    let context = resolve_quality_context(db, input)?;
    let run = db.create_domain_quality_run(&context)?;
    let result = {
        let checks = build_quality_checks(&context);
        let stats = build_quality_stats(&context, &checks);
        let (state, summary) = summarize_quality(&context, &checks);
        db.complete_domain_quality_run(&run.id, state, &summary, stats, checks)
    };
    match result {
        Ok(snapshot) => Ok(snapshot),
        Err(err) => db.fail_domain_quality_run(&run.id, &err.to_string()),
    }
}

pub fn resolve_quality_context(
    db: &SessionDB,
    input: RunDomainQualityInput,
) -> Result<QualityContext> {
    let session_id = non_empty(&input.session_id)
        .ok_or_else(|| anyhow!("session_id is required"))?
        .to_string();
    let session = db
        .get_session(&session_id)?
        .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
    if session.incognito {
        bail!("domain quality is disabled for incognito sessions");
    }

    let goal_id = match input.goal_id.as_deref().and_then(non_empty) {
        Some(goal_id) => {
            let goal = db
                .get_goal(goal_id)?
                .ok_or_else(|| anyhow!("goal not found: {goal_id}"))?;
            if goal.session_id != session_id {
                bail!("goal {} belongs to session {}", goal.id, goal.session_id);
            }
            Some(goal.id)
        }
        None => db.active_goal_id_for_session(&session_id)?,
    };

    let template = resolve_domain_quality_template(db, &input, &session_id, goal_id.as_deref())?;
    let domain = template.domain.clone();
    let evidence = db.list_domain_evidence(ListDomainEvidenceInput {
        goal_id: None,
        session_id: Some(session_id.clone()),
        project_id: None,
        domain: Some(domain.clone()),
        evidence_type: None,
        limit: Some(DOMAIN_QUALITY_LIMIT_DEFAULT),
    })?;
    let (evidence, evidence_scope) = scope_domain_quality_evidence(&input, evidence);

    Ok(QualityContext {
        session_id,
        goal_id,
        domain,
        template,
        evidence,
        evidence_scope,
        input,
    })
}

pub fn resolve_domain_quality_template(
    db: &SessionDB,
    input: &RunDomainQualityInput,
    session_id: &str,
    goal_id: Option<&str>,
) -> Result<DomainWorkflowTemplate> {
    if let Some(template_id) = input.template_id.as_deref().and_then(non_empty) {
        return db
            .get_domain_workflow_template(template_id, input.template_version.as_deref())?
            .ok_or_else(|| anyhow!("domain workflow template not found: {template_id}"));
    }

    let explicit_domain = input
        .domain
        .as_deref()
        .and_then(non_empty)
        .map(normalize_domain);
    if explicit_domain.is_none() {
        if let Some(goal_id) = goal_id {
            if let Some(goal) = db.get_goal(goal_id)? {
                if let Some(template_id) = goal.workflow_template_id.as_deref().and_then(non_empty)
                {
                    return db
                        .get_domain_workflow_template(
                            template_id,
                            goal.workflow_template_version.as_deref(),
                        )?
                        .ok_or_else(|| {
                            anyhow!("domain workflow template not found: {template_id}")
                        });
                }
            }
        }
    }
    let inferred_domain = explicit_domain
        .or_else(|| infer_domain_from_quality_input(db, session_id, goal_id, input).ok());
    let domain = inferred_domain.unwrap_or_else(|| "writing".to_string());
    let templates = db.list_domain_workflow_templates(ListDomainWorkflowTemplatesInput {
        domain: Some(domain.clone()),
        task_type: None,
        project_id: None,
        include_disabled: false,
        limit: Some(1),
    })?;
    templates
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no domain workflow template available for domain {domain}"))
}

fn infer_domain_from_quality_input(
    db: &SessionDB,
    session_id: &str,
    goal_id: Option<&str>,
    input: &RunDomainQualityInput,
) -> Result<String> {
    if let Some(kind) = input.artifact_kind.as_deref().and_then(non_empty) {
        let domain = infer_domain_from_text(kind);
        if domain != "writing" || kind.to_ascii_lowercase().contains("write") {
            return Ok(domain);
        }
    }
    if let Some(goal_id) = goal_id {
        if let Some(goal) = db.get_goal(goal_id)? {
            if let Some(domain) = goal.domain.as_deref().and_then(non_empty) {
                return Ok(normalize_domain(domain));
            }
            let text = format!("{} {}", goal.objective, goal.completion_criteria);
            let lower = text.to_ascii_lowercase();
            let domain = infer_domain_from_text(&text);
            if domain != "writing"
                || contains_any(
                    &lower,
                    &[
                        "writing", "write", "doc", "memo", "report", "文档", "写作", "报告",
                    ],
                )
            {
                return Ok(domain);
            }
        }
    }
    let evidence = db.list_domain_evidence(ListDomainEvidenceInput {
        goal_id: goal_id.map(str::to_string),
        session_id: Some(session_id.to_string()),
        project_id: None,
        domain: None,
        evidence_type: None,
        limit: Some(50),
    })?;
    let mut counts = BTreeMap::<String, usize>::new();
    for item in evidence {
        *counts.entry(item.domain).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(domain, _)| domain)
        .ok_or_else(|| anyhow!("no domain signal available"))
}

#[derive(Debug, Clone, Default)]
struct DomainQualityArtifactTarget {
    id: Option<String>,
    title: Option<String>,
    kind: Option<String>,
    path: Option<String>,
}

fn scope_domain_quality_evidence(
    input: &RunDomainQualityInput,
    evidence: Vec<DomainEvidenceItem>,
) -> (Vec<DomainEvidenceItem>, Value) {
    let total = evidence.len();
    let Some(target) = domain_quality_artifact_target(input) else {
        return (
            evidence,
            json!({
                "enabled": false,
                "mode": "all",
                "total": total,
                "matched": total,
            }),
        );
    };

    let explicit_artifact_evidence = evidence
        .iter()
        .filter(|item| domain_quality_evidence_has_artifact_signal(item))
        .count();
    let target_json = domain_quality_artifact_target_json(&target);
    if explicit_artifact_evidence == 0 {
        return (
            evidence,
            json!({
                "enabled": true,
                "mode": "legacy_fallback_all",
                "target": target_json,
                "total": total,
                "matched": total,
                "explicitArtifactEvidence": explicit_artifact_evidence,
            }),
        );
    }

    let matched = evidence
        .into_iter()
        .filter(|item| domain_quality_evidence_matches_artifact(item, &target))
        .collect::<Vec<_>>();
    let matched_len = matched.len();
    (
        matched,
        json!({
            "enabled": true,
            "mode": "artifact_matched",
            "target": target_json,
            "total": total,
            "matched": matched_len,
            "explicitArtifactEvidence": explicit_artifact_evidence,
        }),
    )
}

fn domain_quality_artifact_target(
    input: &RunDomainQualityInput,
) -> Option<DomainQualityArtifactTarget> {
    let metadata = &input.source_metadata;
    let target = DomainQualityArtifactTarget {
        id: json_string(metadata, "artifactId")
            .or_else(|| json_nested_string(metadata, "artifact", "id")),
        title: input
            .artifact_title
            .as_deref()
            .and_then(non_empty)
            .map(str::to_string)
            .or_else(|| json_string(metadata, "artifactTitle"))
            .or_else(|| json_nested_string(metadata, "artifact", "title")),
        kind: input
            .artifact_kind
            .as_deref()
            .and_then(non_empty)
            .map(normalize_domain)
            .or_else(|| json_string(metadata, "artifactKind").map(|value| normalize_domain(&value)))
            .or_else(|| {
                json_nested_string(metadata, "artifact", "kind")
                    .map(|value| normalize_domain(&value))
            }),
        path: json_string(metadata, "artifactPath")
            .or_else(|| json_nested_string(metadata, "artifact", "path"))
            .or_else(|| json_string(metadata, "path"))
            .or_else(|| json_string(metadata, "filePath"))
            .or_else(|| json_string(metadata, "outputPath"))
            .or_else(|| json_string(metadata, "draftPath")),
    };
    if target.id.is_some()
        || target.title.is_some()
        || target.kind.is_some()
        || target.path.is_some()
    {
        Some(target)
    } else {
        None
    }
}

fn domain_quality_artifact_target_json(target: &DomainQualityArtifactTarget) -> Value {
    json!({
        "id": target.id,
        "title": target.title,
        "kind": target.kind,
        "path": target.path,
    })
}

fn domain_quality_evidence_has_artifact_signal(item: &DomainEvidenceItem) -> bool {
    matches!(
        item.evidence_type.as_str(),
        "artifact_created" | "artifact_reviewed"
    ) || domain_quality_evidence_artifact_id(item).is_some()
        || !domain_quality_evidence_artifact_titles(item).is_empty()
        || domain_quality_evidence_artifact_kind(item).is_some()
        || !domain_quality_evidence_artifact_paths(item).is_empty()
}

fn domain_quality_evidence_matches_artifact(
    item: &DomainEvidenceItem,
    target: &DomainQualityArtifactTarget,
) -> bool {
    if let Some(target_id) = target.id.as_deref() {
        if domain_quality_evidence_artifact_id(item)
            .as_deref()
            .is_some_and(|id| normalized_eq(id, target_id))
        {
            return true;
        }
    }
    if let Some(target_path) = target.path.as_deref() {
        if domain_quality_evidence_artifact_paths(item)
            .iter()
            .any(|path| normalized_path_eq(path, target_path))
        {
            return true;
        }
    }
    if let Some(target_title) = target.title.as_deref() {
        if domain_quality_evidence_artifact_titles(item)
            .iter()
            .any(|title| title_matches(title, target_title))
        {
            return true;
        }
    }
    let has_specific_target =
        target.id.is_some() || target.path.is_some() || target.title.is_some();
    if !has_specific_target {
        if let Some(target_kind) = target.kind.as_deref() {
            if domain_quality_evidence_artifact_kind(item)
                .as_deref()
                .is_some_and(|kind| normalized_eq(kind, target_kind))
                && matches!(
                    item.evidence_type.as_str(),
                    "artifact_created" | "artifact_reviewed"
                )
            {
                return true;
            }
        }
    }
    false
}

fn domain_quality_evidence_artifact_id(item: &DomainEvidenceItem) -> Option<String> {
    json_string(&item.source_metadata, "artifactId")
        .or_else(|| json_nested_string(&item.source_metadata, "artifact", "id"))
}

fn domain_quality_evidence_artifact_titles(item: &DomainEvidenceItem) -> Vec<String> {
    let mut values = Vec::new();
    if matches!(
        item.evidence_type.as_str(),
        "artifact_created" | "artifact_reviewed"
    ) {
        values.push(item.title.clone());
        if let Some(summary) = item.summary.as_deref().and_then(non_empty) {
            values.push(summary.to_string());
        }
    }
    push_optional(
        &mut values,
        json_string(&item.source_metadata, "artifactTitle"),
    );
    push_optional(
        &mut values,
        json_nested_string(&item.source_metadata, "artifact", "title"),
    );
    if matches!(
        item.evidence_type.as_str(),
        "artifact_created" | "artifact_reviewed"
    ) {
        push_optional(&mut values, json_string(&item.source_metadata, "title"));
    }
    values
}

fn domain_quality_evidence_artifact_kind(item: &DomainEvidenceItem) -> Option<String> {
    json_string(&item.source_metadata, "artifactKind")
        .or_else(|| json_nested_string(&item.source_metadata, "artifact", "kind"))
        .map(|value| normalize_domain(&value))
}

fn domain_quality_evidence_artifact_paths(item: &DomainEvidenceItem) -> Vec<String> {
    let mut values = Vec::new();
    for key in [
        "artifactPath",
        "path",
        "filePath",
        "outputPath",
        "draftPath",
    ] {
        push_optional(&mut values, json_string(&item.source_metadata, key));
    }
    push_optional(
        &mut values,
        json_nested_string(&item.source_metadata, "artifact", "path"),
    );
    values
}

fn push_optional(values: &mut Vec<String>, value: Option<String>) {
    if let Some(value) = value.and_then(|value| non_empty(&value).map(str::to_string)) {
        values.push(value);
    }
}

fn json_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(str::to_string)
}

fn json_nested_string(value: &Value, object_key: &str, key: &str) -> Option<String> {
    value
        .get(object_key)
        .and_then(|object| json_string(object, key))
}

fn normalized_eq(left: &str, right: &str) -> bool {
    normalize_domain(left) == normalize_domain(right)
}

fn normalized_path_eq(left: &str, right: &str) -> bool {
    let normalize = |value: &str| value.trim().trim_end_matches('/').to_ascii_lowercase();
    let left = normalize(left);
    let right = normalize(right);
    !left.is_empty() && left == right
}

fn title_matches(left: &str, right: &str) -> bool {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right || left.contains(&right) || right.contains(&left)
}

fn infer_domain_from_text(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if contains_any(
        &lower,
        &[
            "data",
            "metric",
            "kpi",
            "dashboard",
            "chart",
            "数据",
            "指标",
            "图表",
        ],
    ) {
        "data_analysis".to_string()
    } else if contains_any(
        &lower,
        &[
            "email", "inbox", "reply", "message", "gmail", "邮件", "收件", "回复",
        ],
    ) {
        "inbox".to_string()
    } else if contains_any(
        &lower,
        &[
            "meeting", "calendar", "agenda", "attendee", "会议", "日程", "议程",
        ],
    ) {
        "meeting_prep".to_string()
    } else if contains_any(
        &lower,
        &["knowledge", "note", "vault", "obsidian", "知识", "笔记"],
    ) {
        "knowledge_curation".to_string()
    } else if contains_any(
        &lower,
        &[
            "research", "source", "citation", "cite", "调研", "研究", "引用", "资料",
        ],
    ) {
        "research".to_string()
    } else if contains_any(
        &lower,
        &[
            "project", "status", "owner", "deadline", "risk", "项目", "进度", "风险",
        ],
    ) {
        "project_ops".to_string()
    } else {
        "writing".to_string()
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn build_quality_checks(context: &QualityContext) -> Vec<CandidateCheck> {
    let mut checks = Vec::new();
    let counts = evidence_counts(&context.evidence);
    for req in &context.template.required_evidence {
        checks.push(requirement_check(
            req,
            counts.get(&req.evidence_type).copied().unwrap_or(0),
        ));
    }
    checks.extend(domain_profile_checks(context, &counts));
    checks.extend(approval_gate_checks(context));
    dedup_checks(&mut checks);
    checks
}

fn requirement_check(req: &DomainEvidenceRequirement, actual: usize) -> CandidateCheck {
    let min = req.min_count.unwrap_or(1).max(1);
    let ok = actual >= min;
    let required = req.required;
    let status = if ok {
        DomainQualityCheckStatus::Passed
    } else if required {
        DomainQualityCheckStatus::Failed
    } else {
        DomainQualityCheckStatus::Advisory
    };
    let severity = if ok {
        DomainQualitySeverity::P3
    } else if required {
        DomainQualitySeverity::P1
    } else {
        DomainQualitySeverity::P3
    };
    CandidateCheck {
        check_type: "verification".to_string(),
        profile: "required_evidence".to_string(),
        title: req.title.clone(),
        body: if ok {
            format!("{actual}/{min} required evidence item(s) are present.")
        } else if required {
            format!(
                "Missing required evidence: {actual}/{min} item(s) for {}.",
                req.evidence_type
            )
        } else {
            format!(
                "Optional evidence is thin: {actual}/{min} item(s) for {}.",
                req.evidence_type
            )
        },
        severity,
        status,
        evidence_type: Some(req.evidence_type.clone()),
        source_metadata: json!({
            "actual": actual,
            "required": min,
            "requiredEvidence": required,
            "metadataKeys": req.metadata_keys,
        }),
    }
}

fn domain_profile_checks(
    context: &QualityContext,
    counts: &BTreeMap<String, usize>,
) -> Vec<CandidateCheck> {
    match context.domain.as_str() {
        "research" => vec![
            threshold_check("verification", "research", "Key claims have source support", counts, "claim_checked", 2),
            threshold_check("verification", "research", "Sources are cited", counts, "source_cited", 3),
            threshold_check("review", "research", "Citation audit is complete", counts, "citation_audited", 1),
            dated_sources_check(context),
        ],
        "writing" => vec![
            threshold_check("review", "writing", "Draft artifact exists", counts, "artifact_created", 1),
            threshold_check("review", "writing", "Draft reviewed against audience and requirements", counts, "artifact_reviewed", 1),
            advisory_check("review", "writing", "Terminology and audience fit need human judgment", "Review notes should explicitly cover audience, terminology, unmet requirements, and citation gaps."),
        ],
        "data_analysis" => vec![
            threshold_check("verification", "data_analysis", "Data quality checked", counts, "data_quality_checked", 1),
            threshold_check("verification", "data_analysis", "Metric interpretation checked", counts, "claim_checked", 1),
            data_quality_metadata_check(context),
        ],
        "meeting_prep" => vec![
            threshold_check("review", "meeting_prep", "Meeting context collected", counts, "meeting_context_collected", 1),
            threshold_check("review", "meeting_prep", "Brief or agenda exists", counts, "artifact_created", 1),
            advisory_check("review", "meeting_prep", "Decision points and risks surfaced", "Open decisions, risks, missing materials, and unread attachments should be explicit before the meeting."),
        ],
        "inbox" => vec![
            threshold_check("review", "inbox", "Thread or message source cited", counts, "source_cited", 1),
            threshold_check("verification", "inbox", "Facts and commitments checked", counts, "claim_checked", 1),
            threshold_check("approval", "inbox", "Message draft approved before send", counts, "message_draft_approved", 1),
        ],
        "knowledge_curation" => vec![
            threshold_check("verification", "knowledge_curation", "Source notes identified", counts, "source_cited", 2),
            threshold_check("review", "knowledge_curation", "Deduplication and gap review completed", counts, "artifact_reviewed", 1),
            threshold_check("review", "knowledge_curation", "Curated note or index exists", counts, "artifact_created", 1),
        ],
        "project_ops" => vec![
            threshold_check("review", "project_ops", "Status or plan artifact exists", counts, "artifact_created", 1),
            threshold_check("verification", "project_ops", "Risks and dependencies checked", counts, "claim_checked", 1),
            threshold_check("approval", "project_ops", "Owners and tradeoffs confirmed", counts, "user_decision", 1),
        ],
        _ => vec![advisory_check(
            "review",
            &context.domain,
            "General domain review",
            "No specialized profile matched; rely on required evidence and approval gates.",
        )],
    }
}

fn threshold_check(
    check_type: &str,
    profile: &str,
    title: &str,
    counts: &BTreeMap<String, usize>,
    evidence_type: &str,
    min: usize,
) -> CandidateCheck {
    let actual = counts.get(evidence_type).copied().unwrap_or(0);
    let ok = actual >= min;
    CandidateCheck {
        check_type: check_type.to_string(),
        profile: profile.to_string(),
        title: title.to_string(),
        body: if ok {
            format!("{actual}/{min} evidence item(s) satisfy {evidence_type}.")
        } else {
            format!("Need at least {min} {evidence_type} evidence item(s); found {actual}.")
        },
        severity: if ok {
            DomainQualitySeverity::P3
        } else {
            DomainQualitySeverity::P1
        },
        status: if ok {
            DomainQualityCheckStatus::Passed
        } else {
            DomainQualityCheckStatus::Failed
        },
        evidence_type: Some(evidence_type.to_string()),
        source_metadata: json!({ "actual": actual, "required": min }),
    }
}

fn advisory_check(check_type: &str, profile: &str, title: &str, body: &str) -> CandidateCheck {
    CandidateCheck {
        check_type: check_type.to_string(),
        profile: profile.to_string(),
        title: title.to_string(),
        body: body.to_string(),
        severity: DomainQualitySeverity::P3,
        status: DomainQualityCheckStatus::Advisory,
        evidence_type: None,
        source_metadata: json!({}),
    }
}

fn dated_sources_check(context: &QualityContext) -> CandidateCheck {
    let sources = context
        .evidence
        .iter()
        .filter(|item| item.evidence_type == "source_cited")
        .collect::<Vec<_>>();
    let dated = sources
        .iter()
        .filter(|item| {
            item.source_metadata.get("retrievedAt").is_some()
                || item.source_metadata.get("publishedAt").is_some()
                || item.source_metadata.get("date").is_some()
        })
        .count();
    let status = if sources.is_empty() {
        DomainQualityCheckStatus::Failed
    } else if dated == sources.len() {
        DomainQualityCheckStatus::Passed
    } else {
        DomainQualityCheckStatus::Advisory
    };
    CandidateCheck {
        check_type: "verification".to_string(),
        profile: "research".to_string(),
        title: "Source freshness is visible".to_string(),
        body: if status == DomainQualityCheckStatus::Passed {
            "Every cited source includes retrieved/published/date metadata.".to_string()
        } else if status == DomainQualityCheckStatus::Failed {
            "No cited sources are available, so freshness cannot be checked.".to_string()
        } else {
            format!(
                "{dated}/{} cited source(s) include date metadata.",
                sources.len()
            )
        },
        severity: if status == DomainQualityCheckStatus::Failed {
            DomainQualitySeverity::P1
        } else {
            DomainQualitySeverity::P3
        },
        status,
        evidence_type: Some("source_cited".to_string()),
        source_metadata: json!({ "sources": sources.len(), "dated": dated }),
    }
}

fn data_quality_metadata_check(context: &QualityContext) -> CandidateCheck {
    let quality_items = context
        .evidence
        .iter()
        .filter(|item| item.evidence_type == "data_quality_checked")
        .collect::<Vec<_>>();
    let has_definition = quality_items.iter().any(|item| {
        item.source_metadata.get("dataset").is_some()
            || item.source_metadata.get("metric").is_some()
            || item.source_metadata.get("denominator").is_some()
            || item.source_metadata.get("sampleSize").is_some()
    });
    CandidateCheck {
        check_type: "verification".to_string(),
        profile: "data_analysis".to_string(),
        title: "Metric definition and sample context are visible".to_string(),
        body: if has_definition {
            "Data quality evidence includes dataset, metric, denominator, or sample-size metadata."
                .to_string()
        } else {
            "Add dataset, metric definition, denominator, sample size, outlier, or chart-review metadata to the data quality evidence.".to_string()
        },
        severity: if has_definition {
            DomainQualitySeverity::P3
        } else {
            DomainQualitySeverity::P2
        },
        status: if has_definition {
            DomainQualityCheckStatus::Passed
        } else {
            DomainQualityCheckStatus::Advisory
        },
        evidence_type: Some("data_quality_checked".to_string()),
        source_metadata: json!({ "qualityEvidence": quality_items.len(), "hasDefinition": has_definition }),
    }
}

fn approval_gate_checks(context: &QualityContext) -> Vec<CandidateCheck> {
    let requested_action = context
        .input
        .source_metadata
        .get("requestedAction")
        .or_else(|| context.input.source_metadata.get("action"))
        .and_then(Value::as_str)
        .map(normalize_domain);
    let high_risk_requested = context
        .input
        .source_metadata
        .get("highRiskAction")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    context
        .template
        .approval_gates
        .iter()
        .filter(|gate| gate.required)
        .map(|gate| {
            let action = normalize_domain(&gate.action);
            let applies = high_risk_requested
                || requested_action
                    .as_ref()
                    .map(|requested| requested == &action)
                    .unwrap_or(false);
            let status = if context.input.explicit_user_approval {
                DomainQualityCheckStatus::Passed
            } else if applies {
                DomainQualityCheckStatus::NeedsUser
            } else {
                DomainQualityCheckStatus::Advisory
            };
            CandidateCheck {
                check_type: "approval".to_string(),
                profile: "approval_gate".to_string(),
                title: format!("Approval gate: {}", gate.action),
                body: if status == DomainQualityCheckStatus::Passed {
                    "Explicit user approval was provided for this quality run.".to_string()
                } else if status == DomainQualityCheckStatus::NeedsUser {
                    format!(
                        "{} This high-risk action must not proceed without user confirmation.",
                        gate.reason
                    )
                } else {
                    format!(
                        "{} No matching high-risk action is currently requested.",
                        gate.reason
                    )
                },
                severity: if status == DomainQualityCheckStatus::NeedsUser {
                    DomainQualitySeverity::P0
                } else {
                    DomainQualitySeverity::P3
                },
                status,
                evidence_type: Some("user_decision".to_string()),
                source_metadata: json!({
                    "gate": gate.action,
                    "applies": applies,
                    "requestedAction": requested_action,
                    "highRiskAction": high_risk_requested,
                    "explicitUserApproval": context.input.explicit_user_approval,
                }),
            }
        })
        .collect()
}

fn build_quality_stats(context: &QualityContext, checks: &[CandidateCheck]) -> Value {
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut blocked = 0usize;
    let mut needs_user = 0usize;
    let mut advisory = 0usize;
    for check in checks {
        match check.status {
            DomainQualityCheckStatus::Passed => passed += 1,
            DomainQualityCheckStatus::Failed => failed += 1,
            DomainQualityCheckStatus::Blocked => blocked += 1,
            DomainQualityCheckStatus::NeedsUser => needs_user += 1,
            DomainQualityCheckStatus::Advisory => advisory += 1,
        }
    }
    json!({
        "domain": context.domain,
        "templateId": context.template.id,
        "templateVersion": context.template.version,
        "templateTitle": context.template.title,
        "sourceType": context
            .input
            .source_metadata
            .get("sourceType")
            .and_then(Value::as_str)
            .unwrap_or("live"),
        "source": context.input.source_metadata.clone(),
        "profiles": active_profiles(&context.input.profiles, &context.domain),
        "evidence": evidence_counts(&context.evidence),
        "evidenceScope": context.evidence_scope,
        "checks": checks.len(),
        "passed": passed,
        "failed": failed,
        "blocked": blocked,
        "needsUser": needs_user,
        "advisory": advisory,
        "artifact": {
            "title": context.input.artifact_title,
            "kind": context.input.artifact_kind,
        },
    })
}

fn summarize_quality(
    context: &QualityContext,
    checks: &[CandidateCheck],
) -> (DomainQualityRunState, String) {
    let blocking = checks
        .iter()
        .filter(|check| check.status.blocks_goal() && check.severity.is_blocking())
        .count();
    let needs_user = checks
        .iter()
        .filter(|check| check.status == DomainQualityCheckStatus::NeedsUser)
        .count();
    let failed = checks
        .iter()
        .filter(|check| check.status == DomainQualityCheckStatus::Failed)
        .count();
    if blocking > 0 && needs_user > 0 && failed == 0 {
        (
            DomainQualityRunState::NeedsUser,
            format!(
                "{} quality needs user confirmation ({} gate/check).",
                context.template.title, needs_user
            ),
        )
    } else if blocking > 0 {
        (
            DomainQualityRunState::Blocked,
            format!(
                "{} quality blocked by {} required check(s).",
                context.template.title, blocking
            ),
        )
    } else {
        (
            DomainQualityRunState::Completed,
            format!(
                "{} quality checks passed with {} advisory note(s).",
                context.template.title,
                checks
                    .iter()
                    .filter(|check| check.status == DomainQualityCheckStatus::Advisory)
                    .count()
            ),
        )
    }
}

fn evidence_counts(evidence: &[DomainEvidenceItem]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for item in evidence {
        *counts.entry(item.evidence_type.clone()).or_default() += 1;
    }
    counts
}

fn dedup_checks(checks: &mut Vec<CandidateCheck>) {
    let mut seen = Vec::<String>::new();
    checks.retain(|check| {
        let key = format!(
            "{}:{}:{}:{}",
            check.check_type,
            check.profile,
            check.title,
            check.evidence_type.as_deref().unwrap_or("")
        );
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::domain_workflow::RecordDomainEvidenceInput;
    use ha_core::goal::{CreateGoalInput, GoalState};
    use ha_core::session::SessionDB;
    use tempfile::tempdir;

    fn test_db() -> (tempfile::TempDir, SessionDB) {
        // 迁出前这些测试跑在 ha-core 里，`transition_workflow_run` 会**直接**
        // 调 `ensure_coding_workflow_retro_for_run` 记一条终态复盘。现在那条路
        // 走 `improve_hooks`，不 wire 就恒返 `Ok(None)`——不装配等于让这批测试
        // 悄悄跑在一条「永远不记复盘」的路径上，与生产不同。`wire()` 幂等。
        crate::wire();
        let dir = tempdir().expect("tempdir");
        let db =
            SessionDB::open_ephemeral_for_test(&dir.path().join("sessions.db")).expect("open db");
        ensure_channel_conversations_table(&db);
        (dir, db)
    }

    fn ensure_channel_conversations_table(db: &SessionDB) {
        db.with_conn_for_test(|conn| {
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
            .expect("create channel conversations table");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn research_quality_blocks_missing_required_evidence() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "完成 research brief".to_string(),
                completion_criteria: "引用、claim check、citation audit 都齐全".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let snapshot = run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id.clone(),
                goal_id: Some(goal.goal.id.clone()),
                domain: Some("research".to_string()),
                ..Default::default()
            },
        )
        .expect("run quality");

        assert_eq!(snapshot.run.state, DomainQualityRunState::Blocked);
        assert!(snapshot
            .checks
            .iter()
            .any(
                |check| check.evidence_type.as_deref() == Some("source_cited")
                    && check.status == DomainQualityCheckStatus::Failed
            ));
        let goal = db
            .goal_snapshot(&goal.goal.id, 100)
            .expect("goal snapshot")
            .expect("goal exists");
        assert_eq!(goal.goal.state, GoalState::Blocked);
        assert!(goal.evidence.iter().any(|item| {
            item.relation == "domain_quality_blocked" || item.relation == "domain_quality_check"
        }));
    }

    #[test]
    fn domain_quality_prefers_goal_workflow_template() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Prepare a sourced brief".to_string(),
                completion_criteria: "Draft is reviewed and ready to share".to_string(),
                domain: None,
                workflow_template_id: Some("writing-brief".to_string()),
                workflow_template_version: None,
                workflow_task_type: Some("prd".to_string()),
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let snapshot = run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id,
                goal_id: Some(goal.goal.id),
                ..Default::default()
            },
        )
        .expect("run quality");

        assert_eq!(snapshot.run.domain, "writing");
        assert_eq!(snapshot.run.template_id.as_deref(), Some("writing-brief"));
        assert_eq!(snapshot.run.template_version.as_deref(), Some("1.0.0"));
        assert_eq!(
            snapshot
                .run
                .stats
                .get("templateVersion")
                .and_then(Value::as_str),
            Some("1.0.0")
        );
    }

    #[test]
    fn domain_quality_started_event_records_artifact_review_context() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");

        let snapshot = run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id,
                domain: Some("research".to_string()),
                artifact_title: Some("Research brief".to_string()),
                artifact_kind: Some("brief".to_string()),
                source_metadata: json!({
                    "sourceType": "artifact_export_guard",
                    "artifactGuardStatus": "failed",
                }),
                ..Default::default()
            },
        )
        .expect("run quality");

        let started = snapshot
            .events
            .iter()
            .find(|event| event.kind == "domain_quality_started")
            .expect("started event");
        assert_eq!(
            started
                .payload
                .pointer("/artifact/title")
                .and_then(Value::as_str),
            Some("Research brief")
        );
        assert_eq!(
            started
                .payload
                .pointer("/artifact/kind")
                .and_then(Value::as_str),
            Some("brief")
        );
        assert_eq!(
            started
                .payload
                .pointer("/source/sourceType")
                .and_then(Value::as_str),
            Some("artifact_export_guard")
        );
    }

    #[test]
    fn domain_quality_scopes_evidence_to_matching_artifact() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        for evidence_type in ["artifact_created", "artifact_reviewed"] {
            db.record_domain_evidence(RecordDomainEvidenceInput {
                session_id: Some(session.id.clone()),
                domain: "writing".to_string(),
                evidence_type: evidence_type.to_string(),
                title: format!("Other memo {evidence_type}"),
                source_metadata: json!({
                    "artifactTitle": "Other memo",
                    "artifactKind": "memo",
                }),
                ..Default::default()
            })
            .expect("record other artifact evidence");
        }

        let target_missing = run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id.clone(),
                domain: Some("writing".to_string()),
                artifact_title: Some("Target memo".to_string()),
                artifact_kind: Some("memo".to_string()),
                ..Default::default()
            },
        )
        .expect("run target quality");
        assert_eq!(
            target_missing
                .run
                .stats
                .pointer("/evidenceScope/mode")
                .and_then(Value::as_str),
            Some("artifact_matched")
        );
        assert_eq!(
            target_missing
                .run
                .stats
                .pointer("/evidenceScope/matched")
                .and_then(Value::as_u64),
            Some(0)
        );
        assert!(target_missing.checks.iter().any(|check| {
            check.evidence_type.as_deref() == Some("artifact_created")
                && check.status == DomainQualityCheckStatus::Failed
        }));

        for evidence_type in ["artifact_created", "artifact_reviewed"] {
            db.record_domain_evidence(RecordDomainEvidenceInput {
                session_id: Some(session.id.clone()),
                domain: "writing".to_string(),
                evidence_type: evidence_type.to_string(),
                title: format!("Target memo {evidence_type}"),
                source_metadata: json!({
                    "artifactTitle": "Target memo",
                    "artifactKind": "memo",
                }),
                ..Default::default()
            })
            .expect("record target artifact evidence");
        }

        let target_ready = run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id,
                domain: Some("writing".to_string()),
                artifact_title: Some("Target memo".to_string()),
                artifact_kind: Some("memo".to_string()),
                ..Default::default()
            },
        )
        .expect("run target quality with matching evidence");
        assert_eq!(
            target_ready
                .run
                .stats
                .pointer("/evidence/artifact_created")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            target_ready
                .run
                .stats
                .pointer("/evidence/artifact_reviewed")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert!(target_ready.checks.iter().any(|check| {
            check.evidence_type.as_deref() == Some("artifact_created")
                && check.status == DomainQualityCheckStatus::Passed
        }));
    }

    #[test]
    fn domain_quality_falls_back_when_legacy_evidence_has_no_artifact_signal() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session.id.clone()),
            domain: "research".to_string(),
            evidence_type: "source_cited".to_string(),
            title: "Official source".to_string(),
            source_metadata: json!({
                "retrievedAt": "2026-07-03T00:00:00Z",
            }),
            ..Default::default()
        })
        .expect("record legacy evidence");

        let snapshot = run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id,
                domain: Some("research".to_string()),
                artifact_title: Some("Research brief".to_string()),
                artifact_kind: Some("brief".to_string()),
                ..Default::default()
            },
        )
        .expect("run quality");

        assert_eq!(
            snapshot
                .run
                .stats
                .pointer("/evidenceScope/mode")
                .and_then(Value::as_str),
            Some("legacy_fallback_all")
        );
        assert_eq!(
            snapshot
                .run
                .stats
                .pointer("/evidence/source_cited")
                .and_then(Value::as_u64),
            Some(1)
        );
    }

    #[test]
    fn inbox_send_quality_requires_user_confirmation() {
        let (_dir, db) = test_db();
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "draft email reply".to_string(),
                completion_criteria: "recipient facts checked and send approved".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");

        let snapshot = run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id,
                goal_id: Some(goal.goal.id),
                domain: Some("inbox".to_string()),
                source_metadata: json!({
                    "requestedAction": "send_message",
                    "highRiskAction": true,
                }),
                ..Default::default()
            },
        )
        .expect("run quality");

        assert!(snapshot.checks.iter().any(|check| {
            check.check_type == "approval"
                && check.status == DomainQualityCheckStatus::NeedsUser
                && check.severity == DomainQualitySeverity::P0
        }));
        assert!(matches!(
            snapshot.run.state,
            DomainQualityRunState::NeedsUser | DomainQualityRunState::Blocked
        ));
    }
}
