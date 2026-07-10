//! Context Retrieval v2: task-aware read-only context ranking for the
//! workspace panel.
//!
//! This module deliberately aggregates existing owner-plane signals instead of
//! creating another mutable control-plane object. It ranks the files, semantic
//! diagnostics, review findings, verification steps, symbols, and URL sources a
//! user is most likely to need next. Incognito sessions return an empty,
//! explicitly-disabled snapshot.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::domain_workflow::{
    DomainApprovalGate, DomainEvidenceItem, DomainEvidenceRequirement, DomainVerificationRule,
    ListDomainEvidenceInput, ListDomainWorkflowTemplatesInput,
};
use crate::review::{ReviewFindingStatus, ReviewSeverity};
use crate::session::{effective_working_dir_for_meta, SessionDB, SessionIdeContext};
use crate::util::now_rfc3339;
use crate::verification::VerificationStepState;
use crate::workflow::{WorkflowOpState, WorkflowRunState};

const DEFAULT_LIMIT: usize = 24;
const MAX_LIMIT: usize = 50;
const REVIEW_RUN_LIMIT: usize = 3;
const VERIFICATION_RUN_LIMIT: usize = 3;
const FILE_SEARCH_LIMIT: usize = 24;
const SYMBOL_SEARCH_LIMIT: usize = 24;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRetrievalInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ide_context: Option<SessionIdeContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ContextCandidateKind {
    File,
    Symbol,
    Diagnostic,
    ReviewFinding,
    VerificationStep,
    GoalEvidence,
    Task,
    WorkflowOp,
    UrlSource,
    IdeContext,
    Document,
    EmailThread,
    CalendarEvent,
    SheetRange,
    KnowledgeNote,
    WebSource,
    Decision,
    Artifact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCandidate {
    pub id: String,
    pub kind: ContextCandidateKind,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub score: u32,
    pub reasons: Vec<String>,
    pub sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRetrievalStats {
    pub git_changes: usize,
    pub artifact_files: usize,
    pub diagnostics: usize,
    pub review_findings: usize,
    pub verification_steps: usize,
    pub goal_evidence: usize,
    pub tasks: usize,
    pub workflow_ops: usize,
    #[serde(default)]
    pub ide_context_signals: usize,
    pub file_search_matches: usize,
    pub symbols: usize,
    pub url_sources: usize,
    #[serde(default)]
    pub domain_candidates: usize,
    #[serde(default)]
    pub domain_evidence: usize,
    #[serde(default)]
    pub access_issues: usize,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainContextProfile {
    pub domain: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_objective: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_criteria: Option<String>,
    #[serde(default)]
    pub required_evidence: Vec<DomainEvidenceRequirement>,
    #[serde(default)]
    pub approval_gates: Vec<DomainApprovalGate>,
    #[serde(default)]
    pub verification_policy: Vec<DomainVerificationRule>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextAccessIssue {
    pub kind: String,
    pub title: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_connector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRetrievalSnapshot {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    pub candidates: Vec<ContextCandidate>,
    pub stats: ContextRetrievalStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_context: Option<DomainContextProfile>,
    #[serde(default)]
    pub access_issues: Vec<ContextAccessIssue>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    pub generated_at: String,
}

struct CandidateAccum {
    candidate: ContextCandidate,
    rank: i32,
}

#[derive(Debug, Clone)]
struct ResolvedDomainContext {
    profile: DomainContextProfile,
    goal_criteria_tokens: Vec<String>,
}

#[derive(Debug, Clone)]
struct QueryMatcher {
    raw: String,
    tokens: Vec<String>,
}

impl QueryMatcher {
    fn new(query: Option<&str>) -> Self {
        let raw = query.unwrap_or("").trim().to_lowercase();
        let tokens = raw
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        Self { raw, tokens }
    }

    fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    fn boost(&self, fields: &[&str]) -> i32 {
        if self.is_empty() {
            return 0;
        }
        let haystack = fields
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        if haystack.is_empty() {
            return 0;
        }
        let mut boost = 0;
        if haystack.contains(&self.raw) {
            boost += 260;
        }
        let mut matched = 0;
        for token in &self.tokens {
            if haystack.contains(token) {
                matched += 1;
                boost += 55;
            }
        }
        if matched > 0 && matched == self.tokens.len() {
            boost += 160;
        }
        boost
    }
}

pub async fn context_retrieval_for_session(
    db: Arc<SessionDB>,
    session_id: String,
    input: ContextRetrievalInput,
) -> Result<ContextRetrievalSnapshot> {
    let limit = input.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let query = input
        .query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let matcher = QueryMatcher::new(query.as_deref());
    let meta = db
        .get_session(&session_id)?
        .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
    let workspace_root = effective_working_dir_for_meta(&meta);

    if meta.incognito {
        return Ok(ContextRetrievalSnapshot {
            session_id,
            query,
            workspace_root,
            candidates: Vec::new(),
            stats: ContextRetrievalStats::default(),
            domain_context: None,
            access_issues: Vec::new(),
            truncated: false,
            disabled_reason: Some("incognito".to_string()),
            generated_at: now_rfc3339(),
        });
    }

    let mut stats = ContextRetrievalStats::default();
    let mut map: HashMap<String, CandidateAccum> = HashMap::new();
    let ide_context = input.ide_context.clone().or_else(|| {
        db.get_session_ide_context(&session_id)
            .ok()
            .flatten()
            .map(|snapshot| snapshot.context)
    });
    let domain_context = resolve_domain_context(&db, &session_id, &input);

    gather_ide_context(ide_context.as_ref(), &matcher, &mut map, &mut stats);
    gather_git_changes(db.clone(), &session_id, &matcher, &mut map, &mut stats).await;
    gather_artifacts(db.clone(), &session_id, &matcher, &mut map, &mut stats).await;
    gather_lsp_diagnostics(&db, &session_id, &matcher, &mut map, &mut stats).await;
    gather_review_findings(&db, &session_id, &matcher, &mut map, &mut stats);
    gather_verification_steps(&db, &session_id, &matcher, &mut map, &mut stats);
    gather_goal_evidence(&db, &session_id, &matcher, &mut map, &mut stats);
    gather_tasks(&db, &session_id, &matcher, &mut map, &mut stats);
    gather_workflow_ops(&db, &session_id, &matcher, &mut map, &mut stats);
    gather_domain_context(
        &db,
        &session_id,
        domain_context.as_ref(),
        &matcher,
        &mut map,
        &mut stats,
    );
    gather_file_search(
        workspace_root.as_deref(),
        query.as_deref(),
        &matcher,
        &mut map,
        &mut stats,
    )
    .await;
    gather_lsp_symbols(
        &db,
        &session_id,
        query.as_deref(),
        &matcher,
        &mut map,
        &mut stats,
    )
    .await;
    apply_domain_boosts(domain_context.as_ref(), &matcher, &mut map);
    let access_issues = domain_context
        .as_ref()
        .map(|ctx| domain_access_issues(ctx, &map))
        .unwrap_or_default();
    stats.access_issues = access_issues.len();

    let mut candidates = map.into_values().collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.rank
            .cmp(&a.rank)
            .then_with(|| kind_rank(&a.candidate.kind).cmp(&kind_rank(&b.candidate.kind)))
            .then_with(|| a.candidate.title.cmp(&b.candidate.title))
    });
    let truncated = candidates.len() > limit;
    let candidates = candidates
        .into_iter()
        .take(limit)
        .map(|acc| acc.candidate)
        .collect();

    Ok(ContextRetrievalSnapshot {
        session_id,
        query,
        workspace_root,
        candidates,
        stats,
        domain_context: domain_context.map(|ctx| ctx.profile),
        access_issues,
        truncated,
        disabled_reason: None,
        generated_at: now_rfc3339(),
    })
}

fn resolve_domain_context(
    db: &SessionDB,
    session_id: &str,
    input: &ContextRetrievalInput,
) -> Option<ResolvedDomainContext> {
    let goal_snapshot = db
        .active_goal_for_session(session_id)
        .ok()
        .flatten()
        .or_else(|| db.latest_goal_for_session(session_id).ok().flatten());
    let explicit_domain = input
        .domain
        .as_deref()
        .and_then(non_empty)
        .map(normalize_domain_token);
    let goal_domain = goal_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.goal.domain.as_deref())
        .and_then(non_empty)
        .map(normalize_domain_token);
    let workflow_domain = recent_domain_workflow_domain(db, session_id);
    let evidence_domain = recent_domain_evidence_domain(db, session_id);
    let inferred_domain = goal_snapshot.as_ref().and_then(|snapshot| {
        infer_domain_from_goal(&snapshot.goal.objective, &snapshot.goal.completion_criteria)
    });

    let mut source = "none".to_string();
    let mut domain = explicit_domain.inspect(|_| source = "input".to_string());
    if domain.is_none() {
        domain = goal_domain.inspect(|_| source = "goal".to_string());
    }
    if domain.is_none() {
        domain = workflow_domain.inspect(|_| source = "workflow".to_string());
    }
    if domain.is_none() {
        domain = evidence_domain.inspect(|_| source = "domain_evidence".to_string());
    }
    if domain.is_none() {
        domain = inferred_domain.inspect(|_| source = "goal_inference".to_string());
    }

    let template = input
        .template_id
        .as_deref()
        .and_then(non_empty)
        .and_then(|id| {
            db.get_domain_workflow_template(id, input.template_version.as_deref())
                .ok()
                .flatten()
        })
        .or_else(|| {
            goal_snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .goal
                    .workflow_template_id
                    .as_deref()
                    .and_then(non_empty)
                    .and_then(|id| {
                        db.get_domain_workflow_template(
                            id,
                            snapshot.goal.workflow_template_version.as_deref(),
                        )
                        .ok()
                        .flatten()
                    })
            })
        })
        .or_else(|| {
            domain.as_ref().and_then(|domain| {
                db.list_domain_workflow_templates(ListDomainWorkflowTemplatesInput {
                    domain: Some(domain.clone()),
                    limit: Some(1),
                    ..Default::default()
                })
                .ok()
                .and_then(|mut templates| templates.drain(..).next())
            })
        });
    if let Some(template) = template.as_ref() {
        if domain.is_none() {
            domain = Some(template.domain.clone());
            source = "template".to_string();
        }
    }
    let domain = domain?;
    let goal_criteria_tokens = goal_snapshot
        .as_ref()
        .map(|snapshot| {
            tokenize_domain_text(&format!(
                "{}\n{}",
                snapshot.goal.objective, snapshot.goal.completion_criteria
            ))
        })
        .unwrap_or_default();
    let profile = DomainContextProfile {
        domain,
        template_id: template.as_ref().map(|template| template.id.clone()),
        template_version: template.as_ref().map(|template| template.version.clone()),
        template_title: template.as_ref().map(|template| template.title.clone()),
        task_type: template.as_ref().and_then(|template| {
            goal_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.goal.workflow_task_type.clone())
                .or_else(|| template.task_types.first().cloned())
        }),
        goal_id: goal_snapshot
            .as_ref()
            .map(|snapshot| snapshot.goal.id.clone()),
        goal_objective: goal_snapshot
            .as_ref()
            .map(|snapshot| snapshot.goal.objective.clone()),
        completion_criteria: goal_snapshot
            .as_ref()
            .map(|snapshot| snapshot.goal.completion_criteria.clone()),
        required_evidence: template
            .as_ref()
            .map(|template| template.required_evidence.clone())
            .unwrap_or_default(),
        approval_gates: template
            .as_ref()
            .map(|template| template.approval_gates.clone())
            .unwrap_or_default(),
        verification_policy: template
            .as_ref()
            .map(|template| template.verification_policy.clone())
            .unwrap_or_default(),
        source,
    };
    Some(ResolvedDomainContext {
        profile,
        goal_criteria_tokens,
    })
}

fn recent_domain_workflow_domain(db: &SessionDB, session_id: &str) -> Option<String> {
    db.list_workflow_runs_for_session(session_id, 6)
        .ok()?
        .into_iter()
        .find_map(|run| {
            run.kind
                .strip_prefix("domain:")
                .map(normalize_domain_token)
                .filter(|domain| !domain.is_empty())
        })
}

fn recent_domain_evidence_domain(db: &SessionDB, session_id: &str) -> Option<String> {
    db.list_domain_evidence(ListDomainEvidenceInput {
        session_id: Some(session_id.to_string()),
        limit: Some(1),
        ..Default::default()
    })
    .ok()?
    .into_iter()
    .next()
    .map(|item| item.domain)
}

fn infer_domain_from_goal(objective: &str, criteria: &str) -> Option<String> {
    let text = format!("{objective}\n{criteria}").to_ascii_lowercase();
    let checks: &[(&str, &[&str])] = &[
        (
            "data_analysis",
            &[
                "metric",
                "kpi",
                "dashboard",
                "data",
                "数据",
                "指标",
                "报表",
                "分析",
            ],
        ),
        (
            "meeting_prep",
            &[
                "meeting", "agenda", "calendar", "会议", "议程", "参会", "会前",
            ],
        ),
        (
            "inbox",
            &["email", "mail", "reply", "inbox", "邮件", "回复", "收件箱"],
        ),
        (
            "knowledge_curation",
            &["knowledge", "note", "vault", "wiki", "知识", "笔记", "整理"],
        ),
        (
            "writing",
            &[
                "write", "draft", "memo", "prd", "doc", "报告", "文档", "草稿", "写作",
            ],
        ),
        (
            "research",
            &[
                "research", "source", "citation", "market", "调研", "引用", "来源", "竞品",
            ],
        ),
        (
            "project_ops",
            &["project", "status", "risk", "计划", "项目", "风险", "进度"],
        ),
    ];
    checks.iter().find_map(|(domain, needles)| {
        needles
            .iter()
            .any(|needle| text.contains(needle))
            .then(|| domain.to_string())
    })
}

fn gather_domain_context(
    db: &SessionDB,
    session_id: &str,
    domain_context: Option<&ResolvedDomainContext>,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Some(ctx) = domain_context else {
        return;
    };
    let mut evidence = db
        .list_domain_evidence(ListDomainEvidenceInput {
            goal_id: ctx.profile.goal_id.clone(),
            session_id: Some(session_id.to_string()),
            domain: Some(ctx.profile.domain.clone()),
            limit: Some(80),
            ..Default::default()
        })
        .unwrap_or_default();
    if evidence.is_empty() && ctx.profile.goal_id.is_some() {
        evidence = db
            .list_domain_evidence(ListDomainEvidenceInput {
                session_id: Some(session_id.to_string()),
                domain: Some(ctx.profile.domain.clone()),
                limit: Some(80),
                ..Default::default()
            })
            .unwrap_or_default();
    }
    stats.domain_evidence = evidence.len();
    for (idx, item) in evidence.into_iter().enumerate() {
        upsert_domain_evidence_candidate(ctx, item, idx, matcher, map, stats);
    }
    gather_domain_artifacts(db, session_id, ctx, matcher, map, stats);
}

fn upsert_domain_evidence_candidate(
    ctx: &ResolvedDomainContext,
    item: DomainEvidenceItem,
    idx: usize,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let source = item.source_metadata.clone();
    let path = metadata_string(
        &source,
        &["path", "filePath", "file", "artifactPath", "notePath"],
    );
    let url = metadata_string(&source, &["uri", "url", "sourceUrl", "href"]);
    let kind = domain_evidence_kind(&item, path.as_deref(), url.as_deref());
    let subtitle = domain_evidence_subtitle(&item, &source, path.as_deref(), url.as_deref());
    let mut fields = vec![
        item.title.as_str(),
        item.summary.as_deref().unwrap_or_default(),
        item.domain.as_str(),
        item.evidence_type.as_str(),
        path.as_deref().unwrap_or_default(),
        url.as_deref().unwrap_or_default(),
    ];
    fields.extend(ctx.goal_criteria_tokens.iter().map(String::as_str));
    let boost = matcher.boost(&fields);
    let required_boost = if ctx
        .profile
        .required_evidence
        .iter()
        .any(|req| req.evidence_type == item.evidence_type && req.required)
    {
        110
    } else {
        0
    };
    let confidence_boost = item
        .confidence
        .map(|confidence| (confidence.clamp(0.0, 1.0) * 80.0).round() as i32)
        .unwrap_or(20);
    let redaction_penalty = if item.redaction_status == "sensitive" {
        80
    } else {
        0
    };
    let action_metadata = domain_actions_for_candidate(&item.evidence_type, &kind);
    let key = domain_candidate_key(&kind, &item, path.as_deref(), url.as_deref());
    upsert_candidate(
        map,
        key,
        ContextCandidate {
            id: format!("domain-evidence:{}", item.id),
            kind,
            title: item.title.clone(),
            subtitle,
            path,
            line: None,
            url,
            score: 0,
            reasons: Vec::new(),
            sources: Vec::new(),
            status: Some(item.evidence_type.clone()),
            metadata: json!({
                "origin": "domain_evidence",
                "domain": item.domain,
                "evidenceId": item.id,
                "evidenceType": item.evidence_type,
                "goalId": item.goal_id,
                "confidence": item.confidence,
                "accessScope": item.access_scope,
                "redactionStatus": item.redaction_status,
                "sourceMetadata": item.source_metadata,
                "domainActions": action_metadata,
                "staleness": staleness_label(&source),
            }),
        },
        domain_evidence_context_score(&item.evidence_type)
            + required_boost
            + confidence_boost
            + boost
            - redaction_penalty
            - idx.min(60) as i32,
        "Domain workflow 记录了这条通用证据",
        "domain_evidence",
    );
    stats.domain_candidates += 1;
}

fn gather_domain_artifacts(
    db: &SessionDB,
    session_id: &str,
    ctx: &ResolvedDomainContext,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Ok(artifacts) = crate::session::aggregate_session_artifacts(db, session_id) else {
        return;
    };
    for (idx, file) in artifacts.files.into_iter().take(60).enumerate() {
        let Some(kind) = domain_file_kind(&file.path, &ctx.profile.domain) else {
            continue;
        };
        let boost = matcher.boost(&[&file.path, &file.kind, &ctx.profile.domain]);
        let reason = match kind {
            ContextCandidateKind::Document => "当前领域任务可能需要引用这个文档",
            ContextCandidateKind::SheetRange => "当前领域任务可能需要核对这个表格或数据产物",
            ContextCandidateKind::KnowledgeNote => "当前领域任务可能需要这条知识笔记",
            ContextCandidateKind::Artifact => "当前领域任务最近产生或读取过这个产物",
            _ => "当前领域任务可能需要这个上下文",
        };
        upsert_candidate(
            map,
            format!("domain-artifact:{}", file.path),
            ContextCandidate {
                id: format!("domain-artifact:{}", file.path),
                kind: kind.clone(),
                title: display_path(&file.path),
                subtitle: Some(file.path.clone()),
                path: Some(file.path.clone()),
                line: None,
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some(file.kind.clone()),
                metadata: json!({
                    "origin": "domain_artifact",
                    "domain": ctx.profile.domain,
                    "artifactKind": file.kind,
                    "linesAdded": file.lines_added,
                    "linesRemoved": file.lines_removed,
                    "readLines": file.read_lines,
                    "domainActions": domain_actions_for_kind(&kind),
                }),
            },
            domain_artifact_score(&kind, &ctx.profile.domain) + boost - idx.min(50) as i32,
            reason,
            "domain_artifact",
        );
        stats.domain_candidates += 1;
    }
    for (idx, source) in artifacts.sources.into_iter().take(30).enumerate() {
        let boost = matcher.boost(&[&source.url, &source.origin, &ctx.profile.domain]);
        upsert_candidate(
            map,
            format!("domain-web:{}", source.url),
            ContextCandidate {
                id: format!("domain-web:{}", source.url),
                kind: ContextCandidateKind::WebSource,
                title: source
                    .url
                    .split('/')
                    .rfind(|segment| !segment.is_empty())
                    .unwrap_or(source.url.as_str())
                    .to_string(),
                subtitle: Some(source.url.clone()),
                path: None,
                line: None,
                url: Some(source.url.clone()),
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some(source.origin.clone()),
                metadata: json!({
                    "origin": "domain_web_source",
                    "domain": ctx.profile.domain,
                    "sourceOrigin": source.origin,
                    "domainActions": domain_actions_for_kind(&ContextCandidateKind::WebSource),
                }),
            },
            650 + boost - idx.min(30) as i32,
            "当前领域任务引用过这个网页来源",
            "domain_source",
        );
        stats.domain_candidates += 1;
    }
}

fn apply_domain_boosts(
    domain_context: Option<&ResolvedDomainContext>,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
) {
    let Some(ctx) = domain_context else {
        return;
    };
    let domain_terms = domain_terms(&ctx.profile.domain);
    let required_types = ctx
        .profile
        .required_evidence
        .iter()
        .map(|req| req.evidence_type.as_str())
        .collect::<Vec<_>>();
    for acc in map.values_mut() {
        let candidate = &mut acc.candidate;
        let haystack = format!(
            "{}\n{}\n{}\n{}\n{}",
            candidate.title,
            candidate.subtitle.as_deref().unwrap_or_default(),
            candidate.status.as_deref().unwrap_or_default(),
            candidate.path.as_deref().unwrap_or_default(),
            candidate.url.as_deref().unwrap_or_default()
        )
        .to_ascii_lowercase();
        let mut boost = 0;
        if domain_terms.iter().any(|term| haystack.contains(term)) {
            boost += 55;
        }
        if required_types.iter().any(|term| haystack.contains(*term)) {
            boost += 80;
        }
        if !ctx.goal_criteria_tokens.is_empty()
            && ctx
                .goal_criteria_tokens
                .iter()
                .any(|term| term.len() >= 3 && haystack.contains(term))
        {
            boost += 65;
        }
        boost += matcher.boost(&[&ctx.profile.domain]) / 4;
        if boost > 0 {
            acc.rank += boost;
            candidate.score = acc.rank.max(0) as u32;
            add_unique(
                &mut candidate.reasons,
                "命中当前 domain workflow / Goal criteria",
            );
            add_unique(&mut candidate.sources, "domain_ranker");
            if let Some(obj) = candidate.metadata.as_object_mut() {
                obj.insert("domainBoost".to_string(), json!(boost));
                obj.insert("domain".to_string(), json!(ctx.profile.domain.clone()));
            }
        }
    }
}

fn domain_access_issues(
    ctx: &ResolvedDomainContext,
    map: &HashMap<String, CandidateAccum>,
) -> Vec<ContextAccessIssue> {
    let has_kind = |kind: ContextCandidateKind| {
        map.values().any(|acc| {
            acc.candidate.kind == kind
                && acc
                    .candidate
                    .sources
                    .iter()
                    .any(|s| s.starts_with("domain"))
        })
    };
    let has_evidence = |evidence_type: &str| {
        map.values().any(|acc| {
            acc.candidate
                .metadata
                .get("evidenceType")
                .and_then(Value::as_str)
                == Some(evidence_type)
        })
    };
    let mut issues = Vec::new();
    match ctx.profile.domain.as_str() {
        "research" | "writing" if !has_kind(ContextCandidateKind::WebSource) => {
            issues.push(access_issue(
                "web_source",
                "缺少可引用来源",
                "当前 domain workflow 需要可追溯来源；未在会话中看到网页或引用 evidence。",
                Some("web_search"),
                &ctx.profile.domain,
                "连接 Web/Search 或先添加 source_cited evidence",
            ));
        }
        "meeting_prep" if !has_kind(ContextCandidateKind::CalendarEvent) => {
            issues.push(access_issue(
                "calendar_event",
                "缺少会议上下文",
                "会议准备需要日历事件、参会人或材料；当前 snapshot 没有 meeting context evidence。",
                Some("google_calendar"),
                &ctx.profile.domain,
                "连接 Calendar 或记录 meeting_context_collected evidence",
            ));
        }
        "data_analysis"
            if !has_kind(ContextCandidateKind::SheetRange)
                && !has_evidence("data_quality_checked") =>
        {
            issues.push(access_issue(
                "sheet_range",
                "缺少数据源或口径证据",
                "数据分析任务需要表格范围、数据集或 data quality evidence。",
                Some("google_sheets"),
                &ctx.profile.domain,
                "连接 Sheets / 数据源或记录 data_quality_checked evidence",
            ));
        }
        "inbox" if !has_kind(ContextCandidateKind::EmailThread) => {
            issues.push(access_issue(
                "email_thread",
                "缺少邮件线程上下文",
                "邮件沟通任务需要线程、草稿或发送前确认 evidence。",
                Some("gmail"),
                &ctx.profile.domain,
                "连接 Gmail 或记录 message_draft_approved evidence",
            ));
        }
        "knowledge_curation" if !has_kind(ContextCandidateKind::KnowledgeNote) => {
            issues.push(access_issue(
                "knowledge_note",
                "缺少知识笔记上下文",
                "知识整理任务需要 note/source evidence 或知识空间候选。",
                Some("knowledge_base"),
                &ctx.profile.domain,
                "挂载知识空间或记录 source_cited evidence",
            ));
        }
        _ => {}
    }
    for req in &ctx.profile.required_evidence {
        if req.required && !has_evidence(&req.evidence_type) {
            issues.push(access_issue(
                &req.evidence_type,
                &format!("缺少必需证据：{}", req.title),
                "当前 domain workflow 声明了 required evidence，但 snapshot 中还没有对应记录。",
                None,
                &ctx.profile.domain,
                "补齐 evidence 后再完成 Goal",
            ));
        }
    }
    issues
}

fn access_issue(
    kind: &str,
    title: &str,
    reason: &str,
    required_connector: Option<&str>,
    domain: &str,
    action: &str,
) -> ContextAccessIssue {
    ContextAccessIssue {
        kind: kind.to_string(),
        title: title.to_string(),
        reason: reason.to_string(),
        required_connector: required_connector.map(str::to_string),
        domain: Some(domain.to_string()),
        action: action.to_string(),
    }
}

async fn gather_git_changes(
    db: Arc<SessionDB>,
    session_id: &str,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let sid = session_id.to_string();
    let diff =
        tokio::task::spawn_blocking(move || crate::session::load_session_git_diff(&db, &sid)).await;
    let Ok(Ok(diff)) = diff else {
        return;
    };
    stats.git_changes = diff.changes.len();
    for change in diff.changes {
        let action = format!("{:?}", change.action).to_lowercase();
        let line_impact = (change.lines_added + change.lines_removed).min(200) as i32;
        let boost = matcher.boost(&[&change.path, change.language, &action]);
        upsert_candidate(
            map,
            format!("file:{}", change.path),
            ContextCandidate {
                id: format!("file:{}", change.path),
                kind: ContextCandidateKind::File,
                title: display_path(&change.path),
                subtitle: Some(change.path.clone()),
                path: Some(change.path.clone()),
                line: None,
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some(action),
                metadata: json!({
                    "origin": "git_change",
                    "linesAdded": change.lines_added,
                    "linesRemoved": change.lines_removed,
                    "language": change.language,
                    "truncated": change.truncated,
                    "actions": focus_actions(&change.path),
                }),
            },
            900 + line_impact + boost,
            "当前 Git diff 修改过这个文件",
            "git",
        );
    }
}

async fn gather_artifacts(
    db: Arc<SessionDB>,
    session_id: &str,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let sid = session_id.to_string();
    let artifacts =
        tokio::task::spawn_blocking(move || crate::session::aggregate_session_artifacts(&db, &sid))
            .await;
    let Ok(Ok(artifacts)) = artifacts else {
        return;
    };
    stats.artifact_files = artifacts.files.len();
    stats.url_sources = artifacts.sources.len();
    if artifacts.files_truncated {
        stats
            .warnings
            .push("session artifact files were truncated".to_string());
    }
    if artifacts.sources_truncated {
        stats
            .warnings
            .push("session URL sources were truncated".to_string());
    }

    for (idx, file) in artifacts.files.into_iter().take(80).enumerate() {
        let recency = (80usize.saturating_sub(idx).min(80)) as i32;
        let base = if file.kind == "modified" { 735 } else { 610 };
        let boost = matcher.boost(&[&file.path, &file.kind]);
        upsert_candidate(
            map,
            format!("file:{}", file.path),
            ContextCandidate {
                id: format!("file:{}", file.path),
                kind: ContextCandidateKind::File,
                title: display_path(&file.path),
                subtitle: Some(file.path.clone()),
                path: Some(file.path.clone()),
                line: None,
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some(file.kind.clone()),
                metadata: json!({
                    "origin": "session_artifact",
                    "kind": file.kind,
                    "linesAdded": file.lines_added,
                    "linesRemoved": file.lines_removed,
                    "readLines": file.read_lines,
                    "actions": focus_actions(&file.path),
                }),
            },
            base + recency + boost,
            if file.kind == "modified" {
                "本会话最近修改过这个文件"
            } else {
                "本会话最近读取过这个文件"
            },
            "artifacts",
        );
    }

    for (idx, source) in artifacts.sources.into_iter().take(20).enumerate() {
        let boost = matcher.boost(&[&source.url, &source.origin]);
        let title = source
            .url
            .split('/')
            .rfind(|s| !s.is_empty())
            .unwrap_or(source.url.as_str())
            .to_string();
        upsert_candidate(
            map,
            format!("url:{}", source.url),
            ContextCandidate {
                id: format!("url:{}", source.url),
                kind: ContextCandidateKind::UrlSource,
                title,
                subtitle: Some(source.url.clone()),
                path: None,
                line: None,
                url: Some(source.url.clone()),
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some(source.origin.clone()),
                metadata: json!({ "origin": source.origin }),
            },
            430 + (20usize.saturating_sub(idx).min(20) as i32) + boost,
            "本会话引用过这个来源",
            "artifacts",
        );
    }
}

fn gather_ide_context(
    ide: Option<&SessionIdeContext>,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Some(ide) = ide else {
        return;
    };
    if ide.is_empty() {
        return;
    }
    let source = ide.source.as_deref().unwrap_or("ide");
    if let Some(path) = ide.current_file.as_deref() {
        stats.ide_context_signals += 1;
        let boost = matcher.boost(&[path, source]);
        upsert_candidate(
            map,
            format!("file:{path}"),
            ContextCandidate {
                id: format!("file:{path}"),
                kind: ContextCandidateKind::File,
                title: display_path(path),
                subtitle: Some(path.to_string()),
                path: Some(path.to_string()),
                line: None,
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some("current_file".to_string()),
                metadata: json!({
                    "origin": "ide_context",
                    "source": source,
                    "signal": "current_file",
                    "actions": focus_actions(path),
                }),
            },
            960 + boost,
            "IDE/ACP 当前正在查看这个文件",
            "ide",
        );
    }

    if let Some(selection) = ide.selection.as_ref() {
        if let Some(path) = selection.path.as_deref() {
            stats.ide_context_signals += 1;
            let title = selection
                .text
                .as_deref()
                .map(|text| {
                    let compact = text.replace('\n', " ");
                    crate::truncate_utf8(&compact, 80).to_string()
                })
                .unwrap_or_else(|| "IDE selection".to_string());
            let boost = matcher.boost(&[path, &title, source]);
            upsert_candidate(
                map,
                format!(
                    "ide-selection:{path}:{}",
                    selection.start_line.unwrap_or_default()
                ),
                ContextCandidate {
                    id: format!(
                        "ide-selection:{path}:{}",
                        selection.start_line.unwrap_or_default()
                    ),
                    kind: ContextCandidateKind::IdeContext,
                    title,
                    subtitle: Some(path.to_string()),
                    path: Some(path.to_string()),
                    line: selection.start_line,
                    url: None,
                    score: 0,
                    reasons: Vec::new(),
                    sources: Vec::new(),
                    status: Some("selection".to_string()),
                    metadata: json!({
                        "origin": "ide_context",
                        "source": source,
                        "signal": "selection",
                        "range": selection,
                        "actions": focus_actions(path),
                    }),
                },
                990 + boost,
                "IDE/ACP 当前选区直接指向这里",
                "ide",
            );
        }
    }

    if let Some(diagnostic) = ide.active_diagnostic.as_ref() {
        if let Some(path) = diagnostic.path.as_deref() {
            stats.ide_context_signals += 1;
            let title = diagnostic
                .message
                .clone()
                .unwrap_or_else(|| "Active IDE diagnostic".to_string());
            let boost = matcher.boost(&[
                path,
                &title,
                diagnostic.severity.as_deref().unwrap_or_default(),
                source,
            ]);
            upsert_candidate(
                map,
                format!(
                    "ide-diagnostic:{path}:{}",
                    diagnostic.line.unwrap_or_default()
                ),
                ContextCandidate {
                    id: format!(
                        "ide-diagnostic:{path}:{}",
                        diagnostic.line.unwrap_or_default()
                    ),
                    kind: ContextCandidateKind::Diagnostic,
                    title,
                    subtitle: Some(path.to_string()),
                    path: Some(path.to_string()),
                    line: diagnostic.line,
                    url: None,
                    score: 0,
                    reasons: Vec::new(),
                    sources: Vec::new(),
                    status: diagnostic
                        .severity
                        .clone()
                        .or_else(|| Some("diagnostic".to_string())),
                    metadata: json!({
                        "origin": "ide_context",
                        "source": source,
                        "signal": "active_diagnostic",
                        "diagnostic": diagnostic,
                        "actions": focus_actions(path),
                    }),
                },
                980 + boost,
                "IDE/ACP 当前诊断指向这里",
                "ide",
            );
        }
    }

    if let Some(symbol) = ide.active_symbol.as_ref() {
        if let Some(path) = symbol.path.as_deref() {
            stats.ide_context_signals += 1;
            let title = symbol
                .name
                .clone()
                .unwrap_or_else(|| "Active IDE symbol".to_string());
            let boost = matcher.boost(&[
                path,
                &title,
                symbol.kind.as_deref().unwrap_or_default(),
                source,
            ]);
            upsert_candidate(
                map,
                format!("ide-symbol:{path}:{}", symbol.line.unwrap_or_default()),
                ContextCandidate {
                    id: format!("ide-symbol:{path}:{}", symbol.line.unwrap_or_default()),
                    kind: ContextCandidateKind::Symbol,
                    title,
                    subtitle: Some(path.to_string()),
                    path: Some(path.to_string()),
                    line: symbol.line,
                    url: None,
                    score: 0,
                    reasons: Vec::new(),
                    sources: Vec::new(),
                    status: symbol.kind.clone().or_else(|| Some("symbol".to_string())),
                    metadata: json!({
                        "origin": "ide_context",
                        "source": source,
                        "signal": "active_symbol",
                        "symbol": symbol,
                        "actions": focus_actions(path),
                    }),
                },
                940 + boost,
                "IDE/ACP 当前符号与这里相关",
                "ide",
            );
        }
    }

    for (idx, path) in ide.open_tabs.iter().take(12).enumerate() {
        stats.ide_context_signals += 1;
        let boost = matcher.boost(&[path, source]);
        upsert_candidate(
            map,
            format!("file:{path}"),
            ContextCandidate {
                id: format!("file:{path}"),
                kind: ContextCandidateKind::File,
                title: display_path(path),
                subtitle: Some(path.clone()),
                path: Some(path.clone()),
                line: None,
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some("open_tab".to_string()),
                metadata: json!({
                    "origin": "ide_context",
                    "source": source,
                    "signal": "open_tab",
                    "actions": focus_actions(path),
                }),
            },
            720 + boost - idx.min(20) as i32,
            "IDE/ACP 打开的文件提供了当前工作集信号",
            "ide",
        );
    }
}

async fn gather_lsp_diagnostics(
    db: &SessionDB,
    session_id: &str,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Ok(snapshot) = crate::lsp::diagnostics_for_session(db, session_id).await else {
        return;
    };
    stats.diagnostics = snapshot.diagnostics.len();
    for (idx, diagnostic) in snapshot.diagnostics.into_iter().take(80).enumerate() {
        let path = diagnostic
            .path
            .clone()
            .unwrap_or_else(|| diagnostic.uri.clone());
        let source = diagnostic
            .source
            .clone()
            .unwrap_or_else(|| "lsp".to_string());
        let severity_score = match diagnostic.severity.as_str() {
            "error" => 890,
            "warning" => 805,
            "information" => 690,
            "hint" => 625,
            _ => 580,
        };
        let boost = matcher.boost(&[
            &path,
            &diagnostic.message,
            &diagnostic.severity,
            source.as_str(),
        ]);
        upsert_candidate(
            map,
            format!(
                "diagnostic:{}:{}:{}",
                path, diagnostic.range.start_line, diagnostic.range.start_column
            ),
            ContextCandidate {
                id: format!(
                    "diagnostic:{}:{}:{}",
                    path, diagnostic.range.start_line, diagnostic.range.start_column
                ),
                kind: ContextCandidateKind::Diagnostic,
                title: diagnostic.message.clone(),
                subtitle: Some(path.clone()),
                path: Some(path.clone()),
                line: Some(diagnostic.range.start_line),
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some(diagnostic.severity.clone()),
                metadata: json!({
                    "origin": "lsp_diagnostic",
                    "source": source,
                    "code": diagnostic.code,
                    "range": diagnostic.range,
                    "actions": focus_actions(&path),
                }),
            },
            severity_score - idx.min(50) as i32 + boost,
            "语言服务报告了这里的诊断",
            "lsp",
        );
    }
}

fn gather_review_findings(
    db: &SessionDB,
    session_id: &str,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Ok(runs) = db.list_review_runs_for_session(session_id, REVIEW_RUN_LIMIT) else {
        return;
    };
    let mut seen = HashSet::new();
    for run in runs {
        let Ok(findings) = db.list_review_findings_for_run(&run.id) else {
            continue;
        };
        for finding in findings {
            if !seen.insert(finding.id.clone()) {
                continue;
            }
            stats.review_findings += 1;
            let base = review_score(finding.severity, finding.status);
            let boost = matcher.boost(&[
                &finding.file,
                &finding.title,
                &finding.body,
                &finding.category,
                finding.severity.as_str(),
                finding.status.as_str(),
            ]);
            upsert_candidate(
                map,
                format!("review:{}", finding.id),
                ContextCandidate {
                    id: format!("review:{}", finding.id),
                    kind: ContextCandidateKind::ReviewFinding,
                    title: finding.title,
                    subtitle: Some(finding.file.clone()),
                    path: Some(finding.file.clone()),
                    line: finding.start_line,
                    url: None,
                    score: 0,
                    reasons: Vec::new(),
                    sources: Vec::new(),
                    status: Some(format!(
                        "{}:{}:{}",
                        finding.severity.as_str(),
                        finding.verdict.as_str(),
                        finding.status.as_str()
                    )),
                    metadata: json!({
                        "origin": "review_finding",
                        "runId": run.id,
                        "findingId": finding.id,
                        "severity": finding.severity,
                        "verdict": finding.verdict,
                        "status": finding.status,
                        "category": finding.category,
                        "body": finding.body,
                        "actions": focus_actions(&finding.file),
                    }),
                },
                base + boost,
                "代码审查把这里标成待关注项",
                "review",
            );
        }
    }
}

fn gather_verification_steps(
    db: &SessionDB,
    session_id: &str,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Ok(runs) = db.list_verification_runs_for_session(session_id, VERIFICATION_RUN_LIMIT) else {
        return;
    };
    let mut seen = HashSet::new();
    for run in runs {
        let Ok(steps) = db.list_verification_steps_for_run(&run.id) else {
            continue;
        };
        for step in steps {
            if !seen.insert(step.id.clone()) {
                continue;
            }
            stats.verification_steps += 1;
            let base = verification_score(step.state);
            let boost = matcher.boost(&[
                &step.command,
                &step.title,
                &step.reason,
                &step.category,
                step.state.as_str(),
                step.risk.as_str(),
            ]);
            upsert_candidate(
                map,
                format!("verification:{}", step.id),
                ContextCandidate {
                    id: format!("verification:{}", step.id),
                    kind: ContextCandidateKind::VerificationStep,
                    title: step.title,
                    subtitle: Some(step.command.clone()),
                    path: None,
                    line: None,
                    url: None,
                    score: 0,
                    reasons: Vec::new(),
                    sources: Vec::new(),
                    status: Some(step.state.as_str().to_string()),
                    metadata: json!({
                        "origin": "verification_step",
                        "runId": run.id,
                        "stepId": step.id,
                        "command": step.command,
                        "cwd": step.cwd,
                        "reason": step.reason,
                        "category": step.category,
                        "risk": step.risk,
                        "autoRun": step.auto_run,
                        "exitCode": step.exit_code,
                        "outputPreview": step.output_preview,
                        "durationMs": step.duration_ms,
                    }),
                },
                base + boost,
                "验证计划或结果提示这里需要关注",
                "verification",
            );
        }
    }
}

fn gather_goal_evidence(
    db: &SessionDB,
    session_id: &str,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let snapshot = match db.active_goal_for_session(session_id) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => match db.latest_goal_for_session(session_id) {
            Ok(Some(snapshot)) => snapshot,
            _ => return,
        },
        Err(_) => return,
    };
    stats.goal_evidence = snapshot.evidence.len();
    for (idx, item) in snapshot.evidence.iter().rev().take(24).enumerate() {
        let summary = item.summary.clone().unwrap_or_default();
        let path = path_from_metadata(&item.metadata);
        let source_type = item.source_type.clone();
        let source_id = item.source_id.clone();
        let relation = item.relation.clone();
        let boost = matcher.boost(&[
            &item.title,
            &summary,
            &source_type,
            &relation,
            path.as_deref().unwrap_or_default(),
        ]);
        let mut metadata = json!({
            "origin": "goal_evidence",
            "goalId": snapshot.goal.id.clone(),
            "sourceType": source_type,
            "sourceId": source_id,
            "relation": relation,
            "evidenceMetadata": item.metadata.clone(),
        });
        if let Some(path) = path.as_deref() {
            metadata["actions"] = focus_actions(path);
        }
        upsert_candidate(
            map,
            format!("goal-evidence:{}", item.id),
            ContextCandidate {
                id: format!("goal-evidence:{}", item.id),
                kind: ContextCandidateKind::GoalEvidence,
                title: item.title.clone(),
                subtitle: item.summary.clone(),
                path,
                line: None,
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some(item.relation.clone()),
                metadata,
            },
            goal_evidence_score(&item.relation) + boost - idx.min(40) as i32,
            "当前 Goal 把它记录为完成标准证据",
            "goal",
        );
    }
}

fn gather_tasks(
    db: &SessionDB,
    session_id: &str,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Ok(tasks) = db.list_tasks(session_id) else {
        return;
    };
    stats.tasks = tasks.len();
    for (idx, task) in tasks.iter().rev().take(24).enumerate() {
        let title = task
            .active_form
            .as_deref()
            .unwrap_or(task.content.as_str())
            .to_string();
        let boost = matcher.boost(&[
            &title,
            &task.content,
            &task.status,
            task.batch_id.as_deref().unwrap_or_default(),
        ]);
        upsert_candidate(
            map,
            format!("task:{}", task.id),
            ContextCandidate {
                id: format!("task:{}", task.id),
                kind: ContextCandidateKind::Task,
                title,
                subtitle: Some(task.content.clone()),
                path: None,
                line: None,
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: Some(task.status.clone()),
                metadata: json!({
                    "origin": "task",
                    "taskId": task.id,
                    "batchId": task.batch_id.clone(),
                    "createdAt": task.created_at.clone(),
                    "updatedAt": task.updated_at.clone(),
                }),
            },
            task_score(&task.status) + boost - idx.min(40) as i32,
            "当前任务进度与下一步上下文相关",
            "task",
        );
    }
}

fn gather_workflow_ops(
    db: &SessionDB,
    session_id: &str,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Ok(runs) = db.list_workflow_runs_for_session(session_id, 3) else {
        return;
    };
    for run in runs {
        let Ok(Some(snapshot)) = db.workflow_run_snapshot(&run.id, 40) else {
            continue;
        };
        stats.workflow_ops += snapshot.ops.len();
        if snapshot.ops.is_empty() {
            let title = format!("Workflow {} run", snapshot.run.kind);
            let boost = matcher.boost(&[
                &title,
                snapshot.run.state.as_str(),
                &snapshot.run.kind,
                &snapshot.run.execution_mode,
                snapshot.run.blocked_reason.as_deref().unwrap_or_default(),
            ]);
            upsert_candidate(
                map,
                format!("workflow-run:{}", snapshot.run.id),
                ContextCandidate {
                    id: format!("workflow-run:{}", snapshot.run.id),
                    kind: ContextCandidateKind::WorkflowOp,
                    title,
                    subtitle: snapshot.run.blocked_reason.clone(),
                    path: None,
                    line: None,
                    url: None,
                    score: 0,
                    reasons: Vec::new(),
                    sources: Vec::new(),
                    status: Some(snapshot.run.state.as_str().to_string()),
                    metadata: json!({
                        "origin": "workflow_run",
                        "runId": snapshot.run.id.clone(),
                        "kind": snapshot.run.kind.clone(),
                        "executionMode": snapshot.run.execution_mode.clone(),
                        "goalId": snapshot.run.goal_id.clone(),
                        "worktreeId": snapshot.run.worktree_id.clone(),
                    }),
                },
                workflow_run_score(snapshot.run.state) + boost,
                "最近 Workflow Run 是当前执行轨迹的一部分",
                "workflow",
            );
            continue;
        }

        for (idx, op) in snapshot.ops.iter().rev().take(24).enumerate() {
            let error = op.error.as_ref().map(Value::to_string).unwrap_or_default();
            let output = op.output.as_ref().map(Value::to_string).unwrap_or_default();
            let boost = matcher.boost(&[
                &op.op_key,
                &op.op_type,
                op.state.as_str(),
                &error,
                &output,
                &snapshot.run.kind,
            ]);
            upsert_candidate(
                map,
                format!("workflow-op:{}", op.id),
                ContextCandidate {
                    id: format!("workflow-op:{}", op.id),
                    kind: ContextCandidateKind::WorkflowOp,
                    title: format!("{} · {}", op.op_type, op.op_key),
                    subtitle: op.child_handle.clone().or_else(|| {
                        if error.is_empty() {
                            None
                        } else {
                            Some(error.clone())
                        }
                    }),
                    path: None,
                    line: None,
                    url: None,
                    score: 0,
                    reasons: Vec::new(),
                    sources: Vec::new(),
                    status: Some(op.state.as_str().to_string()),
                    metadata: json!({
                        "origin": "workflow_op",
                        "runId": snapshot.run.id.clone(),
                        "runState": snapshot.run.state,
                        "opId": op.id.clone(),
                        "opKey": op.op_key.clone(),
                        "opType": op.op_type.clone(),
                        "effectClass": op.effect_class,
                        "childHandle": op.child_handle.clone(),
                        "error": op.error.clone(),
                        "output": op.output.clone(),
                    }),
                },
                workflow_op_score(op.state) + boost - idx.min(40) as i32,
                "最近 Workflow Op 影响当前长任务执行状态",
                "workflow",
            );
        }
    }
}

async fn gather_file_search(
    workspace_root: Option<&str>,
    query: Option<&str>,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Some(root) = workspace_root.map(str::to_string) else {
        return;
    };
    let Some(query) = query.map(str::to_string) else {
        return;
    };
    let search = tokio::task::spawn_blocking(move || {
        crate::filesystem::search_files(&root, &query, Some(FILE_SEARCH_LIMIT))
    })
    .await;
    let Ok(Ok(response)) = search else {
        return;
    };
    stats.file_search_matches = response.matches.len();
    if response.truncated {
        stats
            .warnings
            .push("file search reached the walk cap".to_string());
    }
    for file in response.matches {
        let boost = matcher.boost(&[&file.path, &file.rel_path, &file.name]);
        upsert_candidate(
            map,
            format!("file:{}", file.path),
            ContextCandidate {
                id: format!("file:{}", file.path),
                kind: ContextCandidateKind::File,
                title: file.name.clone(),
                subtitle: Some(file.rel_path.clone()),
                path: Some(file.path.clone()),
                line: None,
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: if file.is_dir {
                    Some("directory".to_string())
                } else {
                    Some("file".to_string())
                },
                metadata: json!({
                    "origin": "file_search",
                    "relPath": file.rel_path,
                    "isDir": file.is_dir,
                    "fileSearchScore": file.score,
                    "actions": if file.is_dir { Value::Null } else { focus_actions(&file.path) },
                }),
            },
            510 + (file.score / 80).clamp(0, 260) + boost,
            "文件名或路径匹配当前查询",
            "file_search",
        );
    }
}

async fn gather_lsp_symbols(
    db: &SessionDB,
    session_id: &str,
    query: Option<&str>,
    matcher: &QueryMatcher,
    map: &mut HashMap<String, CandidateAccum>,
    stats: &mut ContextRetrievalStats,
) {
    let Some(query) = query else {
        return;
    };
    if query.chars().count() < 2 {
        return;
    }
    let Ok(snapshot) =
        crate::lsp::workspace_symbols_for_session(db, session_id, query, Some(SYMBOL_SEARCH_LIMIT))
            .await
    else {
        return;
    };
    stats.symbols = snapshot.symbols.len();
    for warning in snapshot.errors {
        stats.warnings.push(warning);
    }
    for (idx, symbol) in snapshot.symbols.into_iter().enumerate() {
        let path = symbol.path.clone();
        let line = symbol.range.as_ref().map(|r| r.start_line);
        let detail = symbol.detail.clone().unwrap_or_default();
        let boost = matcher.boost(&[
            &symbol.name,
            &detail,
            symbol.kind.as_deref().unwrap_or("symbol"),
            path.as_deref().unwrap_or(""),
            &symbol.server,
        ]);
        upsert_candidate(
            map,
            format!(
                "symbol:{}:{}:{}",
                symbol.name,
                path.as_deref().unwrap_or(""),
                line.unwrap_or(0)
            ),
            ContextCandidate {
                id: format!(
                    "symbol:{}:{}:{}",
                    symbol.name,
                    path.as_deref().unwrap_or(""),
                    line.unwrap_or(0)
                ),
                kind: ContextCandidateKind::Symbol,
                title: symbol.name,
                subtitle: symbol
                    .kind
                    .clone()
                    .or(symbol.detail.clone())
                    .or_else(|| path.clone()),
                path: path.clone(),
                line,
                url: None,
                score: 0,
                reasons: Vec::new(),
                sources: Vec::new(),
                status: symbol.kind.clone(),
                metadata: json!({
                    "origin": "lsp_symbol",
                    "server": symbol.server,
                    "kind": symbol.kind,
                    "detail": symbol.detail,
                    "range": symbol.range,
                    "actions": path.as_deref().map(focus_actions).unwrap_or(Value::Null),
                }),
            },
            700 + boost - idx.min(50) as i32,
            "语义符号匹配当前查询",
            "lsp",
        );
    }
}

fn upsert_candidate(
    map: &mut HashMap<String, CandidateAccum>,
    key: String,
    mut candidate: ContextCandidate,
    rank: i32,
    reason: &str,
    source: &str,
) {
    candidate.score = rank.max(0) as u32;
    candidate.reasons.push(reason.to_string());
    candidate.sources.push(source.to_string());
    if let Some(existing) = map.get_mut(&key) {
        if rank > existing.rank {
            let mut reasons = existing.candidate.reasons.clone();
            let mut sources = existing.candidate.sources.clone();
            add_unique(&mut reasons, reason);
            add_unique(&mut sources, source);
            candidate.reasons = reasons;
            candidate.sources = sources;
            candidate.score = rank.max(0) as u32;
            existing.candidate = candidate;
            existing.rank = rank;
        } else {
            add_unique(&mut existing.candidate.reasons, reason);
            add_unique(&mut existing.candidate.sources, source);
        }
        return;
    }
    map.insert(key, CandidateAccum { candidate, rank });
}

fn add_unique(list: &mut Vec<String>, value: &str) {
    if !list.iter().any(|item| item == value) {
        list.push(value.to_string());
    }
}

fn domain_evidence_kind(
    item: &DomainEvidenceItem,
    path: Option<&str>,
    url: Option<&str>,
) -> ContextCandidateKind {
    match item.evidence_type.as_str() {
        "source_cited" => {
            if url.is_some() {
                ContextCandidateKind::WebSource
            } else if path.map(is_knowledge_path).unwrap_or(false)
                || metadata_string(&item.source_metadata, &["noteId", "noteTitle", "kbId"])
                    .is_some()
            {
                ContextCandidateKind::KnowledgeNote
            } else {
                ContextCandidateKind::Document
            }
        }
        "user_decision" => ContextCandidateKind::Decision,
        "artifact_created" | "artifact_reviewed" => path
            .and_then(|path| domain_file_kind(path, &item.domain))
            .unwrap_or(ContextCandidateKind::Artifact),
        "data_quality_checked" => ContextCandidateKind::SheetRange,
        "citation_audited" => ContextCandidateKind::WebSource,
        "message_draft_approved" => ContextCandidateKind::EmailThread,
        "meeting_context_collected" => ContextCandidateKind::CalendarEvent,
        "claim_checked" => {
            if url.is_some() {
                ContextCandidateKind::WebSource
            } else {
                ContextCandidateKind::GoalEvidence
            }
        }
        _ => ContextCandidateKind::GoalEvidence,
    }
}

fn domain_evidence_subtitle(
    item: &DomainEvidenceItem,
    source: &Value,
    path: Option<&str>,
    url: Option<&str>,
) -> Option<String> {
    url.map(str::to_string)
        .or_else(|| path.map(str::to_string))
        .or_else(|| {
            metadata_string(
                source,
                &["title", "threadId", "eventId", "dataset", "sheet", "range"],
            )
        })
        .or_else(|| item.summary.clone())
}

fn domain_candidate_key(
    kind: &ContextCandidateKind,
    item: &DomainEvidenceItem,
    path: Option<&str>,
    url: Option<&str>,
) -> String {
    if let Some(url) = url {
        return format!("domain:{kind:?}:url:{url}");
    }
    if let Some(path) = path {
        return format!("domain:{kind:?}:path:{path}");
    }
    format!("domain:{kind:?}:{}", item.id)
}

fn domain_evidence_context_score(evidence_type: &str) -> i32 {
    match evidence_type {
        "source_cited" => 760,
        "claim_checked" => 800,
        "user_decision" => 845,
        "artifact_created" => 700,
        "artifact_reviewed" => 790,
        "data_quality_checked" => 830,
        "citation_audited" => 805,
        "message_draft_approved" => 850,
        "meeting_context_collected" => 820,
        _ => 680,
    }
}

fn domain_artifact_score(kind: &ContextCandidateKind, domain: &str) -> i32 {
    match (kind, domain) {
        (ContextCandidateKind::SheetRange, "data_analysis") => 735,
        (ContextCandidateKind::WebSource, "research") => 725,
        (ContextCandidateKind::Document, "writing") => 710,
        (ContextCandidateKind::KnowledgeNote, "knowledge_curation") => 730,
        (ContextCandidateKind::Artifact, _) => 650,
        _ => 610,
    }
}

fn domain_file_kind(path: &str, domain: &str) -> Option<ContextCandidateKind> {
    let lower = path.to_ascii_lowercase();
    if is_knowledge_path(&lower) {
        return Some(ContextCandidateKind::KnowledgeNote);
    }
    if lower.ends_with(".csv")
        || lower.ends_with(".tsv")
        || lower.ends_with(".xlsx")
        || lower.ends_with(".xls")
        || lower.ends_with(".numbers")
        || lower.contains("sheet")
    {
        return Some(ContextCandidateKind::SheetRange);
    }
    if lower.ends_with(".md")
        || lower.ends_with(".txt")
        || lower.ends_with(".docx")
        || lower.ends_with(".doc")
        || lower.ends_with(".pdf")
        || lower.ends_with(".pptx")
        || lower.ends_with(".pages")
    {
        return Some(ContextCandidateKind::Document);
    }
    if matches!(
        domain,
        "writing" | "research" | "meeting_prep" | "project_ops"
    ) && (lower.contains("brief") || lower.contains("memo") || lower.contains("report"))
    {
        return Some(ContextCandidateKind::Artifact);
    }
    None
}

fn is_knowledge_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/knowledge/")
        || lower.contains("/notes/")
        || lower.contains("/vault/")
        || lower.contains("obsidian")
}

fn domain_actions_for_candidate(evidence_type: &str, kind: &ContextCandidateKind) -> Value {
    let mut actions = domain_actions_for_kind(kind);
    if let Some(obj) = actions.as_object_mut() {
        obj.insert("canAddEvidence".to_string(), json!(true));
        if evidence_type == "claim_checked" {
            obj.insert("canMarkConflict".to_string(), json!(true));
        }
        if matches!(evidence_type, "user_decision" | "message_draft_approved") {
            obj.insert("needsUserConfirmation".to_string(), json!(true));
        }
    }
    actions
}

fn domain_actions_for_kind(kind: &ContextCandidateKind) -> Value {
    json!({
        "canCite": matches!(
            kind,
            ContextCandidateKind::Document
                | ContextCandidateKind::WebSource
                | ContextCandidateKind::KnowledgeNote
                | ContextCandidateKind::SheetRange
        ),
        "canSummarize": matches!(
            kind,
            ContextCandidateKind::Document
                | ContextCandidateKind::WebSource
                | ContextCandidateKind::KnowledgeNote
                | ContextCandidateKind::EmailThread
                | ContextCandidateKind::CalendarEvent
                | ContextCandidateKind::SheetRange
        ),
        "canAskUser": matches!(
            kind,
            ContextCandidateKind::Decision
                | ContextCandidateKind::EmailThread
                | ContextCandidateKind::CalendarEvent
        ),
        "canCreateTask": true,
    })
}

fn staleness_label(source: &Value) -> Option<String> {
    let retrieved = metadata_string(
        source,
        &["retrievedAt", "retrieved_at", "timestamp", "date"],
    )?;
    if retrieved.len() >= 10 {
        Some(format!("retrieved:{retrieved}"))
    } else {
        Some("undated".to_string())
    }
}

fn metadata_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = value.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn tokenize_domain_text(value: &str) -> Vec<String> {
    value
        .to_ascii_lowercase()
        .split(|ch: char| {
            !ch.is_alphanumeric() && ch != '_' && !('\u{4e00}'..='\u{9fff}').contains(&ch)
        })
        .map(str::trim)
        .filter(|token| token.chars().count() >= 2)
        .take(24)
        .map(str::to_string)
        .collect()
}

fn domain_terms(domain: &str) -> &'static [&'static str] {
    match domain {
        "research" => &[
            "research", "source", "citation", "claim", "调研", "来源", "引用",
        ],
        "writing" => &["draft", "doc", "memo", "artifact", "写作", "文档", "草稿"],
        "data_analysis" => &["data", "metric", "kpi", "sheet", "数据", "指标", "口径"],
        "meeting_prep" => &["meeting", "agenda", "calendar", "会议", "议程", "参会"],
        "knowledge_curation" => &["knowledge", "note", "tag", "知识", "笔记", "索引"],
        "inbox" => &["email", "reply", "thread", "邮件", "回复", "线程"],
        "project_ops" => &["project", "status", "risk", "owner", "项目", "风险", "进度"],
        _ => &[],
    }
}

fn normalize_domain_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn review_score(severity: ReviewSeverity, status: ReviewFindingStatus) -> i32 {
    let base = match severity {
        ReviewSeverity::P0 => 985,
        ReviewSeverity::P1 => 935,
        ReviewSeverity::P2 => 855,
        ReviewSeverity::P3 => 720,
    };
    match status {
        ReviewFindingStatus::Open => base,
        ReviewFindingStatus::Resolved => base - 260,
        ReviewFindingStatus::Dismissed | ReviewFindingStatus::FalsePositive => base - 340,
    }
}

fn verification_score(state: VerificationStepState) -> i32 {
    match state {
        VerificationStepState::Failed | VerificationStepState::TimedOut => 910,
        VerificationStepState::Running => 820,
        VerificationStepState::Pending => 735,
        VerificationStepState::Skipped => 650,
        VerificationStepState::Passed => 520,
    }
}

fn goal_evidence_score(relation: &str) -> i32 {
    let lower = relation.to_ascii_lowercase();
    if lower.contains("block") || lower.contains("fail") || lower.contains("open") {
        925
    } else if lower.contains("review") || lower.contains("verification") {
        805
    } else if lower.contains("completed") || lower.contains("pass") {
        670
    } else {
        720
    }
}

fn task_score(status: &str) -> i32 {
    match status {
        "in_progress" => 835,
        "pending" => 760,
        "completed" => 520,
        _ => 650,
    }
}

fn workflow_run_score(state: WorkflowRunState) -> i32 {
    match state {
        WorkflowRunState::Failed | WorkflowRunState::Blocked => 930,
        WorkflowRunState::AwaitingApproval | WorkflowRunState::AwaitingUser => 875,
        WorkflowRunState::Running | WorkflowRunState::Paused | WorkflowRunState::Recovering => 820,
        WorkflowRunState::Draft => 610,
        WorkflowRunState::Completed => 540,
        WorkflowRunState::Cancelled => 460,
    }
}

fn workflow_op_score(state: WorkflowOpState) -> i32 {
    match state {
        WorkflowOpState::Failed => 920,
        WorkflowOpState::Started => 835,
        WorkflowOpState::Pending => 760,
        WorkflowOpState::Completed => 535,
    }
}

fn kind_rank(kind: &ContextCandidateKind) -> u8 {
    match kind {
        ContextCandidateKind::ReviewFinding => 0,
        ContextCandidateKind::Diagnostic => 1,
        ContextCandidateKind::IdeContext => 2,
        ContextCandidateKind::VerificationStep => 3,
        ContextCandidateKind::WorkflowOp => 4,
        ContextCandidateKind::GoalEvidence => 5,
        ContextCandidateKind::Task => 6,
        ContextCandidateKind::Decision => 7,
        ContextCandidateKind::WebSource => 8,
        ContextCandidateKind::Document => 9,
        ContextCandidateKind::KnowledgeNote => 10,
        ContextCandidateKind::CalendarEvent => 11,
        ContextCandidateKind::EmailThread => 12,
        ContextCandidateKind::SheetRange => 13,
        ContextCandidateKind::Artifact => 14,
        ContextCandidateKind::File => 15,
        ContextCandidateKind::Symbol => 16,
        ContextCandidateKind::UrlSource => 17,
    }
}

fn display_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

fn focus_actions(path: &str) -> Value {
    json!({
        "canReview": true,
        "canVerify": true,
        "focusPaths": [path],
    })
}

fn path_from_metadata(metadata: &Value) -> Option<String> {
    for key in ["path", "file", "filePath", "targetPath", "relPath"] {
        if let Some(value) = metadata.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    metadata
        .get("paths")
        .and_then(Value::as_array)
        .and_then(|paths| paths.iter().find_map(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_workflow::RecordDomainEvidenceInput;
    use crate::goal::CreateGoalInput;
    use tempfile::tempdir;

    struct TestDb {
        _dir: tempfile::TempDir,
        db: Arc<SessionDB>,
    }

    fn test_db() -> TestDb {
        let dir = tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open db");
        {
            let conn = db.conn.lock().expect("lock connection");
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
        }
        TestDb {
            _dir: dir,
            db: Arc::new(db),
        }
    }

    #[tokio::test]
    async fn domain_context_retrieval_surfaces_sources_and_access_issues() {
        let test = test_db();
        let db = test.db.clone();
        let session_id = db.create_session("ha-main").expect("create session").id;
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session_id.clone(),
                objective: "Research competitors and produce a cited brief".to_string(),
                completion_criteria: "Needs citations and checked claims".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");
        db.record_domain_evidence(RecordDomainEvidenceInput {
            goal_id: Some(goal.goal.id.clone()),
            domain: "research".to_string(),
            evidence_type: "source_cited".to_string(),
            title: "Official pricing page".to_string(),
            summary: Some("Primary source for competitor pricing".to_string()),
            source_metadata: json!({
                "title": "Pricing",
                "uri": "https://example.com/pricing",
                "retrievedAt": "2026-07-03T00:00:00Z"
            }),
            confidence: Some(0.92),
            access_scope: Some("public".to_string()),
            redaction_status: Some("none".to_string()),
            ..Default::default()
        })
        .expect("record domain evidence");

        let snapshot = context_retrieval_for_session(
            db,
            session_id.clone(),
            ContextRetrievalInput {
                query: Some("pricing".to_string()),
                limit: Some(20),
                ..Default::default()
            },
        )
        .await
        .expect("context retrieval");

        assert_eq!(snapshot.session_id, session_id);
        assert_eq!(
            snapshot
                .domain_context
                .as_ref()
                .map(|ctx| ctx.domain.as_str()),
            Some("research")
        );
        assert!(snapshot.candidates.iter().any(|candidate| {
            candidate.kind == ContextCandidateKind::WebSource
                && candidate.url.as_deref() == Some("https://example.com/pricing")
                && candidate
                    .sources
                    .iter()
                    .any(|source| source == "domain_evidence")
        }));
        assert!(snapshot
            .access_issues
            .iter()
            .any(|issue| issue.kind == "claim_checked" || issue.kind == "citation_audited"));
    }
}
