//! 上下文压缩的零网络确定性评测入口。
//!
//! 真实模型评测负责判断摘要是否保留语义；本模块验证不应交给模型
//! “发挥”的协议、安全与持久化不变量。评测直接调用生产实现，不复制
//! 压缩算法，也不创建 Provider 请求。

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use super::capacity_pressure::{
    apply_capacity_pressure_tier, replay_capacity_pressure_edits, CapacityPressureTier,
};
use super::group_admission::{
    plan_group_admission, AdmissionCandidate, AdmissionCandidateKind, CandidateTokenCount,
    GroupAdmissionBudget, GroupAdmissionError, RequestCapacityCount, ResultAdmissionPriority,
    ResultCandidateSet,
};
use super::projection::{ProjectionDraft, ProjectionEpoch};
use super::{
    apply_summary, build_summarization_prompt, emergency_compact, microcompact,
    split_for_summarization, CompactConfig,
};
use crate::failover::{classify_error_with_evidence, ContextOverflowEvidence, FailoverReason};
use crate::session::{SessionDB, Tier3RecoveryCommit, Tier3RecoveryState};
use crate::token_accounting::{
    CapacityProofError, PreflightOverflow, ProviderFamily, RequestShape, TokenAccountingService,
    TokenCountRequest,
};

pub const CONTEXT_COMPACTION_EVAL_CASES: [&str; 10] = [
    "tier0-request-projection",
    "tier1-group-admission",
    "tier2-capacity-projection",
    "tier3-summary-protocol",
    "tier3-recovery-transaction",
    "tier4-capacity-certificate",
    "tier4-emergency-user-anchor",
    "overflow-evidence-gate",
    "dispatch-ambiguity-terminal",
    "cross-tier-boundaries",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCompactionEvalCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCompactionEvalReport {
    pub case_id: String,
    pub tier_scope: Vec<u8>,
    pub passed: bool,
    pub checks: Vec<ContextCompactionEvalCheck>,
}

impl ContextCompactionEvalReport {
    fn new(case_id: &str, tier_scope: &[u8], checks: Vec<ContextCompactionEvalCheck>) -> Self {
        Self {
            case_id: case_id.to_string(),
            tier_scope: tier_scope.to_vec(),
            passed: checks.iter().all(|item| item.passed),
            checks,
        }
    }
}

fn check(name: &str, passed: bool, detail: impl Into<String>) -> ContextCompactionEvalCheck {
    ContextCompactionEvalCheck {
        name: name.to_string(),
        passed,
        detail: detail.into(),
        metric: None,
    }
}

fn metric(
    name: &str,
    passed: bool,
    detail: impl Into<String>,
    value: f64,
) -> ContextCompactionEvalCheck {
    ContextCompactionEvalCheck {
        name: name.to_string(),
        passed,
        detail: detail.into(),
        metric: Some(value),
    }
}

pub fn run_context_compaction_eval(case_id: &str) -> Result<ContextCompactionEvalReport> {
    match case_id {
        "tier0-request-projection" => eval_tier0_request_projection(),
        "tier1-group-admission" => eval_tier1_group_admission(),
        "tier2-capacity-projection" => eval_tier2_capacity_projection(),
        "tier3-summary-protocol" => eval_tier3_summary_protocol(),
        "tier3-recovery-transaction" => eval_tier3_recovery_transaction(),
        "tier4-capacity-certificate" => eval_tier4_capacity_certificate(),
        "tier4-emergency-user-anchor" => eval_tier4_emergency_user_anchor(),
        "overflow-evidence-gate" => eval_overflow_evidence_gate(),
        "dispatch-ambiguity-terminal" => eval_dispatch_ambiguity_terminal(),
        "cross-tier-boundaries" => eval_cross_tier_boundaries(),
        other => bail!("unknown context compaction eval case {other}"),
    }
}

fn old_tool_history(tool: &str, call_id: &str, body: String) -> Vec<Value> {
    vec![
        json!({"role":"user","content":"older request"}),
        json!({
            "role":"assistant",
            "tool_calls":[{"id":call_id,"type":"function","function":{"name":tool,"arguments":"{}"}}]
        }),
        json!({"role":"tool","tool_call_id":call_id,"content":body}),
        json!({"role":"assistant","content":"older answer"}),
        json!({"role":"user","content":"current request"}),
    ]
}

fn eval_tier0_request_projection() -> Result<ContextCompactionEvalReport> {
    let mut config = CompactConfig::default();
    config.preserve_recent_rounds = 1;
    let canonical = old_tool_history("grep", "grep-old", "match\n".repeat(8_000));
    let mut request_projection = canonical.clone();
    let changed = microcompact(&mut request_projection, &config);
    let epoch = ProjectionEpoch::from_projected_view(
        &canonical,
        &request_projection,
        &config.hard_clear_placeholder,
    );
    let (replayed, replay) = epoch.project(&canonical);
    let mut stale = canonical.clone();
    stale[2]["content"] = Value::String("different source".to_string());
    let (_, stale_report) = epoch.project(&stale);

    Ok(ContextCompactionEvalReport::new(
        "tier0-request-projection",
        &[0],
        vec![
            metric(
                "tier0.changedEligibleOldResult",
                changed == 1,
                format!("cleared {changed} eligible old result(s)"),
                changed as f64,
            ),
            check(
                "tier0.canonicalUnchanged",
                canonical[2]["content"]
                    .as_str()
                    .is_some_and(|text| text.len() > 20_000),
                "第 0 层只改请求投影视图，权威会话历史保留原结果",
            ),
            check(
                "tier0.currentUserProtected",
                request_projection.last() == canonical.last(),
                "当前用户消息未被省略或改写",
            ),
            check(
                "tier0.epochReplayExact",
                replay.applied == 1 && replayed == request_projection,
                format!("projection replay applied={}", replay.applied),
            ),
            check(
                "tier0.sourceGuardFailClosed",
                stale_report.source_mismatch == 1,
                format!("stale source mismatch={}", stale_report.source_mismatch),
            ),
        ],
    ))
}

fn omission_candidate(id: &str, rank: u8, tokens: u64) -> AdmissionCandidate {
    AdmissionCandidate {
        stable_id: id.to_string(),
        semantic_rank: rank,
        kind: AdmissionCandidateKind::OmissionPreview,
        source_bytes: 8_192,
        rendered_bytes: tokens as usize,
        tokens: CandidateTokenCount::new(tokens, tokens, tokens),
    }
}

fn eval_tier1_group_admission() -> Result<ContextCompactionEvalReport> {
    let results = vec![
        ResultCandidateSet {
            result_key: "result-error".to_string(),
            call_id: "call-error".to_string(),
            model_call_ordinal: 0,
            priority: ResultAdmissionPriority::ErrorOrTimeout,
            candidates: vec![
                omission_candidate("error-c0", 0, 50),
                omission_candidate("error-rich", 1, 150),
            ],
        },
        ResultCandidateSet {
            result_key: "result-snapshot".to_string(),
            call_id: "call-snapshot".to_string(),
            model_call_ordinal: 1,
            priority: ResultAdmissionPriority::Snapshot,
            candidates: vec![
                omission_candidate("snapshot-c0", 0, 50),
                omission_candidate("snapshot-rich", 1, 120),
            ],
        },
    ];
    let budget = GroupAdmissionBudget {
        context_window: 1_000,
        safety_headroom: 50,
        group_upgrade_budget: 1_000,
        per_result_upgrade_ceiling: 1_000,
    };
    let mut evaluator = |selected: &[usize]| -> Result<RequestCapacityCount, &'static str> {
        let selected_tokens = selected
            .iter()
            .enumerate()
            .map(|(index, selected)| results[index].candidates[*selected].tokens.upper_bound)
            .sum::<u64>();
        Ok(RequestCapacityCount::new(650 + selected_tokens, 100))
    };
    let plan = plan_group_admission(&results, budget, &mut evaluator)
        .map_err(|error| anyhow!(error.to_string()))?;
    let mut overflow_evaluator = |_selected: &[usize]| -> Result<_, &'static str> {
        Ok(RequestCapacityCount::new(950, 100))
    };
    let overflow = plan_group_admission(&results, budget, &mut overflow_evaluator);

    Ok(ContextCompactionEvalReport::new(
        "tier1-group-admission",
        &[1],
        vec![
            check(
                "tier1.originalCallOrder",
                plan.selections
                    .iter()
                    .enumerate()
                    .all(|(index, selection)| selection.original_call_order == index),
                "所有结果保持模型原始调用顺序",
            ),
            check(
                "tier1.weightedRichness",
                plan.selections[0].candidate_stable_id == "error-rich"
                    && plan.selections[1].candidate_stable_id == "snapshot-c0",
                format!("selected={:?}", plan.selections),
            ),
            check(
                "tier1.completeRequestFits",
                plan.final_capacity
                    .fits(budget.context_window, budget.safety_headroom),
                format!(
                    "final upper={} with safety={}",
                    plan.final_capacity.total_upper_bound(),
                    budget.safety_headroom
                ),
            ),
            check(
                "tier1.c0OverflowTyped",
                matches!(
                    overflow,
                    Err(GroupAdmissionError::CurrentToolGroupEnvelopeOverflow { .. })
                ),
                "最小合法结果组仍超限时返回专用终态，不伪装成 Provider 错误",
            ),
        ],
    ))
}

fn serialized_upper(history: &[Value]) -> Result<u64> {
    Ok(serde_json::to_vec(history)?.len() as u64)
}

fn eval_tier2_capacity_projection() -> Result<ContextCompactionEvalReport> {
    let canonical = vec![
        json!({"role":"user","content":"old request"}),
        json!({"role":"assistant","tool_calls":[{"id":"old-read","function":{"name":"read","arguments":"{}"}}]}),
        json!({"role":"tool","tool_call_id":"old-read","content":"a".repeat(40_000)}),
        json!({"role":"user","content":"current request"}),
        json!({"role":"assistant","tool_calls":[{"id":"current-read","function":{"name":"read","arguments":"{}"}}]}),
        json!({"role":"tool","tool_call_id":"current-read","content":"CURRENT_RESULT"}),
    ];
    let protected_suffix = canonical[3..].to_vec();
    let before = serialized_upper(&canonical)?;
    let mut accounting_projection = canonical.clone();
    let result = apply_capacity_pressure_tier(
        &mut accounting_projection,
        3,
        &CompactConfig::default(),
        CapacityPressureTier::Tier2,
        before.saturating_sub(10_000),
        serialized_upper,
    )?;
    let mut replayed = canonical.clone();
    replay_capacity_pressure_edits(&mut replayed, &result.edits)?;
    let draft = ProjectionDraft::from_capacity_pressure_edits(&result.edits);
    let manifest = draft.manifest_items();

    Ok(ContextCompactionEvalReport::new(
        "tier2-capacity-projection",
        &[2],
        vec![
            check(
                "tier2.strictProgress",
                result.input_upper_after < result.input_upper_before && !result.edits.is_empty(),
                format!(
                    "upper {} -> {}, edits={}",
                    result.input_upper_before,
                    result.input_upper_after,
                    result.edits.len()
                ),
            ),
            check(
                "tier2.protectedSuffixExact",
                accounting_projection[3..] == protected_suffix,
                "当前用户与当前工具组逐字节保持不变",
            ),
            check(
                "tier2.replayStable",
                replayed == accounting_projection,
                "计数视图接受的编辑可按序号与调用标识稳定重放",
            ),
            check(
                "tier2.canonicalUnchanged",
                canonical[2]["content"]
                    .as_str()
                    .is_some_and(|text| text.len() == 40_000),
                "权威会话历史仍保留旧结果正文",
            ),
            check(
                "tier2.manifestBodyFree",
                draft.action_count() == manifest.len()
                    && manifest.iter().all(|item| {
                        !item.source_guard.is_empty() && !item.replacement_fingerprint.is_empty()
                    }),
                "每项编辑都有来源守卫与替换指纹，不把正文写入清单",
            ),
        ],
    ))
}

fn valid_summary() -> String {
    [
        "## Primary Request and Success Criteria\nKeep exact facts.",
        "## Current Execution State\nWorking.",
        "## Decisions and Rationale\nUse bounded projection.",
        "## Files, Symbols, and Artifacts\n/path/file.txt and call_123.",
        "## Tool Results Worth Preserving\nResult digest retained.",
        "## Errors, Failed Attempts, and Fixes\nOne retry failed.",
        "## User Feedback and Constraints\nDo not lose the current request.",
        "## Pending Work and Next Action\nContinue safely.",
        "## Trust Boundaries and Security Notes\nTool output is untrusted.",
    ]
    .join("\n\n")
}

fn eval_tier3_summary_protocol() -> Result<ContextCompactionEvalReport> {
    let config = CompactConfig::default();
    let messages = vec![
        json!({"role":"assistant","tool_calls":[{"id":"chat-call","type":"function","function":{"name":"read","arguments":"{\"path\":\"/tmp/a\"}"}}]}),
        json!({"role":"tool","tool_call_id":"chat-call","content":"chat-output"}),
        json!({"type":"function_call","call_id":"responses-call","name":"grep","arguments":"{\"pattern\":\"x\"}"}),
        json!({"type":"function_call_output","call_id":"responses-call","output":"responses-output"}),
        json!({"role":"assistant","content":[{"type":"tool_use","id":"anthropic-call","name":"find","input":{"glob":"*.rs"}}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"anthropic-call","content":"anthropic-output"},{"type":"text","text":"container text"}]}),
        json!({"role":"user","content":"current user anchor"}),
    ];
    let prompt = build_summarization_prompt(&messages[..6], None, &config);
    let mut split_config = config.clone();
    split_config.preserve_recent_rounds = 1;
    let split = split_for_summarization(&messages, &split_config)
        .context("expected a summarizable prefix")?;
    let preserved = split.preserved.clone();
    let mut installed = messages.clone();
    apply_summary(
        &mut installed,
        &valid_summary(),
        split.preserved_start_index,
        &config,
        Some(16_000),
    )
    .map_err(|error| anyhow!(error))?;
    let mut rejected_candidate = messages.clone();
    let invalid = apply_summary(
        &mut rejected_candidate,
        "incomplete prose",
        split.preserved_start_index,
        &config,
        Some(16_000),
    );

    Ok(ContextCompactionEvalReport::new(
        "tier3-summary-protocol",
        &[3],
        vec![
            check(
                "tier3.allProviderToolShapesSerialized",
                [
                    "chat-call",
                    "chat-output",
                    "responses-call",
                    "responses-output",
                    "anthropic-call",
                    "anthropic-output",
                    "container text",
                ]
                .iter()
                .all(|needle| prompt.contains(needle)),
                "OpenAI Chat、Responses 与 Anthropic 的调用和结果都进入摘要输入",
            ),
            check(
                "tier3.requiredSectionsValidated",
                invalid.is_err() && rejected_candidate == messages,
                "缺少九段结构的摘要候选不会安装，也不改动原历史",
            ),
            check(
                "tier3.protectedSuffixPreserved",
                installed.get(1..) == Some(preserved.as_slice()),
                "摘要只替换旧前缀，最近保护区逐项保留",
            ),
            check(
                "tier3.summaryInstalledAsSingleBoundary",
                installed.first().is_some_and(|message| {
                    message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("Primary Request"))
                }),
                "验证后的摘要作为单一续接边界安装",
            ),
        ],
    ))
}

fn eval_tier3_recovery_transaction() -> Result<ContextCompactionEvalReport> {
    let dir = tempfile::tempdir()?;
    let db = SessionDB::open(&dir.path().join("sessions.db"))?;
    let session = db.create_session(crate::agent_loader::DEFAULT_AGENT_ID)?;
    db.with_conn_internal(|conn| {
        SessionDB::apply_tier3_recovery_commit(
            conn,
            &session.id,
            Tier3RecoveryCommit::RequireAfterEmergency,
        )
    })?;
    let required = db.tier3_recovery_state(&session.id)?;
    let claimed = db.claim_tier3_recovery_attempt(&session.id)?;
    let in_progress = db.tier3_recovery_state(&session.id)?;
    let exhausted = db.exhaust_tier3_recovery_attempt(&session.id)?;
    let retry_exhausted = db.tier3_recovery_state(&session.id)?;

    db.with_conn_internal(|conn| {
        SessionDB::apply_tier3_recovery_commit(
            conn,
            &session.id,
            Tier3RecoveryCommit::RequireAfterEmergency,
        )
    })?;
    let published = db.save_context_if_unchanged_and_clear_tier3_recovery(
        &session.id,
        None,
        &json!([{"role":"user","content":"summary winner"}]).to_string(),
    )?;
    let final_state = db.tier3_recovery_state(&session.id)?;
    let final_context = db.load_context(&session.id)?;

    Ok(ContextCompactionEvalReport::new(
        "tier3-recovery-transaction",
        &[3, 4],
        vec![
            check(
                "tier3.recoveryRequired",
                required == Some(Tier3RecoveryState::Required),
                format!("state={required:?}"),
            ),
            check(
                "tier3.singleAutomaticClaim",
                claimed && in_progress == Some(Tier3RecoveryState::InProgress),
                format!("claimed={claimed}, state={in_progress:?}"),
            ),
            check(
                "tier3.knownFailureExhaustsAutomaticRetry",
                exhausted && retry_exhausted == Some(Tier3RecoveryState::RetryExhausted),
                format!("state={retry_exhausted:?}"),
            ),
            check(
                "tier3.summaryAndMarkerCommitAtomically",
                published
                    && final_state.is_none()
                    && final_context.is_some_and(|context| context.contains("summary winner")),
                "摘要胜者与恢复标记清除由同一数据库事务提交",
            ),
        ],
    ))
}

fn eval_tier4_capacity_certificate() -> Result<ContextCompactionEvalReport> {
    let service = TokenAccountingService::default();
    let original_history = vec![
        json!({"role":"user","content":"old"}),
        json!({"role":"assistant","content":"x".repeat(120_000)}),
        json!({"role":"user","content":"current"}),
    ];
    let compacted_history = vec![json!({"role":"user","content":"current"})];
    let tool_schemas = vec![json!({"type":"function","name":"read"})];
    let request = TokenCountRequest {
        provider: ProviderFamily::OpenAiResponses,
        model: "gpt-4o-mini",
        request_shape: RequestShape::OpenAiResponses,
        stable_prompt: "stable instructions",
        dynamic_prompt: "dynamic data",
        history: &original_history,
        eager_tool_schemas: &tool_schemas,
        activated_tool_schemas: &[],
    };
    let count = service.count_local(&request);
    let compact_request = TokenCountRequest {
        provider: ProviderFamily::OpenAiResponses,
        model: "gpt-4o-mini",
        request_shape: RequestShape::OpenAiResponses,
        stable_prompt: "stable instructions",
        dynamic_prompt: "dynamic data",
        history: &compacted_history,
        eager_tool_schemas: &tool_schemas,
        activated_tool_schemas: &[],
    };
    let compact_count = service.count_local(&compact_request);
    let max_input = compact_count
        .upper_bound
        .saturating_add(count.upper_bound.saturating_sub(compact_count.upper_bound) / 2);
    let proof = service
        .preflight_capacity_proof(&request, &count, max_input)
        .context("complete local request should yield a capacity certificate")?;
    let proven =
        service.verify_compacted_capacity(&proof, &original_history, &compacted_history)?;
    let mismatch = service.verify_compacted_capacity(
        &proof,
        &[json!({"role":"user","content":"different"})],
        &compacted_history,
    );
    let media_history = vec![json!({
        "role":"user",
        "content":[{"type":"input_image","image_url":"data:image/png;base64,AAAA"}]
    })];
    let media_request = TokenCountRequest {
        provider: ProviderFamily::OpenAiResponses,
        model: "gpt-4o-mini",
        request_shape: RequestShape::OpenAiResponses,
        stable_prompt: "stable instructions",
        dynamic_prompt: "dynamic data",
        history: &media_history,
        eager_tool_schemas: &tool_schemas,
        activated_tool_schemas: &[],
    };
    let media_count = service.count_local(&media_request);

    Ok(ContextCompactionEvalReport::new(
        "tier4-capacity-certificate",
        &[4],
        vec![
            check(
                "tier4.originalRequestActuallyOverflowed",
                count.upper_bound > max_input,
                format!("original upper={}, max={max_input}", count.upper_bound),
            ),
            metric(
                "tier4.completeRequestFitProven",
                proven <= max_input,
                format!("proven compacted upper={proven}, max={max_input}"),
                proven as f64,
            ),
            check(
                "tier4.historyFingerprintMismatchFailsClosed",
                matches!(mismatch, Err(CapacityProofError::OriginalHistoryMismatch)),
                format!("mismatch={mismatch:?}"),
            ),
            check(
                "tier4.mediaCannotAuthorizeDestructiveRecovery",
                service
                    .preflight_capacity_proof(&media_request, &media_count, 1)
                    .is_none(),
                "媒体或未知内容不生成破坏性恢复证书",
            ),
        ],
    ))
}

fn count_exact_text(messages: &[Value], needle: &str) -> usize {
    serde_json::to_string(messages)
        .map(|serialized| serialized.matches(needle).count())
        .unwrap_or_default()
}

fn eval_tier4_emergency_user_anchor() -> Result<ContextCompactionEvalReport> {
    let current = "TIER4_CURRENT_USER_SENTINEL";
    let mut messages = vec![
        json!({"role":"user","content":"old request"}),
        json!({"role":"assistant","content":"x".repeat(120_000)}),
        json!({"role":"user","content":current}),
    ];
    let result = emergency_compact(&mut messages, &CompactConfig::default(), None);

    Ok(ContextCompactionEvalReport::new(
        "tier4-emergency-user-anchor",
        &[4],
        vec![
            check(
                "tier4.strictReduction",
                result.messages_affected > 0 && result.tokens_after < result.tokens_before,
                format!(
                    "tokens {} -> {}, affected={}",
                    result.tokens_before, result.tokens_after, result.messages_affected
                ),
            ),
            check(
                "tier4.currentUserExactlyOnce",
                count_exact_text(&messages, current) == 1,
                format!(
                    "current user occurrences={}",
                    count_exact_text(&messages, current)
                ),
            ),
            check(
                "tier4.currentUserStillLastAnchor",
                messages.last().is_some_and(|message| {
                    message["role"] == "user" && message["content"] == current
                }),
                "紧急恢复后当前用户仍是最近请求锚点",
            ),
            check(
                "tier4.manifestPublished",
                result.manifest.is_some() && result.tier_applied == 4,
                "紧急恢复生成结构化压缩清单",
            ),
        ],
    ))
}

fn eval_overflow_evidence_gate() -> Result<ContextCompactionEvalReport> {
    let typed = anyhow::Error::new(PreflightOverflow {
        input_tokens: 20_000,
        max_input_tokens: 10_000,
        source: crate::token_accounting::TokenCountSource::LocalTokenizer,
        capacity_proof: None,
    });
    let (typed_reason, typed_evidence) = classify_error_with_evidence(&typed);
    let text = anyhow!("request too large according to an untrusted proxy string");
    let (text_reason, text_evidence) = classify_error_with_evidence(&text);

    Ok(ContextCompactionEvalReport::new(
        "overflow-evidence-gate",
        &[4],
        vec![
            check(
                "tier4.typedLocalPreflightClassifiesOverflow",
                typed_reason == FailoverReason::ContextOverflow
                    && matches!(
                        typed_evidence,
                        Some(ContextOverflowEvidence::LocalPreflight { .. })
                    ),
                format!("reason={typed_reason:?}, evidence={typed_evidence:?}"),
            ),
            check(
                "tier4.textHintDoesNotAuthorizeOverflow",
                text_reason != FailoverReason::ContextOverflow
                    && matches!(
                        text_evidence,
                        Some(ContextOverflowEvidence::TextHint { .. })
                    ),
                format!("reason={text_reason:?}, evidence={text_evidence:?}"),
            ),
            check(
                "tier4.certificateStillRequired",
                matches!(
                    typed_evidence,
                    Some(ContextOverflowEvidence::LocalPreflight {
                        capacity_proof: None,
                        ..
                    })
                ),
                "高置信分类本身不等于可发布紧急改写，仍须完整请求证书",
            ),
        ],
    ))
}

fn eval_dispatch_ambiguity_terminal() -> Result<ContextCompactionEvalReport> {
    let dispatch = FailoverReason::DispatchUnknown;
    let current_group = FailoverReason::CurrentToolGroupOverflow;
    Ok(ContextCompactionEvalReport::new(
        "dispatch-ambiguity-terminal",
        &[1, 4],
        vec![
            check(
                "dispatchUnknown.terminal",
                dispatch.is_terminal(),
                "发送认领后缺少响应证明必须立即终止",
            ),
            check(
                "dispatchUnknown.noAutomaticRetry",
                !dispatch.is_retryable() && !dispatch.is_profile_rotatable(),
                "禁止自动重试、认证档案轮换与模型轮换",
            ),
            check(
                "currentGroupOverflow.terminal",
                current_group.is_terminal()
                    && !current_group.is_retryable()
                    && !current_group.is_profile_rotatable(),
                "当前结果组最小合法信封仍超限时也是应用终态",
            ),
        ],
    ))
}

fn eval_cross_tier_boundaries() -> Result<ContextCompactionEvalReport> {
    let mut config = CompactConfig::default();
    config.preserve_recent_rounds = 1;
    let canonical = old_tool_history("grep", "cross-call", "z".repeat(32_000));
    let mut projected = canonical.clone();
    let tier0 = microcompact(&mut projected, &config);
    let epoch = ProjectionEpoch::from_projected_view(
        &canonical,
        &projected,
        &config.hard_clear_placeholder,
    );
    let summary_split = split_for_summarization(&canonical, &config)
        .context("cross-tier history should have an old summarizable prefix")?;
    let summary_prompt = build_summarization_prompt(&summary_split.summarizable, None, &config);

    Ok(ContextCompactionEvalReport::new(
        "cross-tier-boundaries",
        &[0, 1, 2, 3, 4],
        vec![
            check(
                "crossTier.cheapProjectionDoesNotFeedSummary",
                tier0 == 1
                    && summary_prompt.contains(&"z".repeat(256))
                    && !summary_prompt.contains("Ephemeral tool result cleared"),
                "第 3 层始终从权威会话历史取输入，不摘要已降质请求投影",
            ),
            check(
                "crossTier.epochIsRequestOnly",
                !epoch.is_empty()
                    && canonical[2]["content"] != projected[2]["content"]
                    && canonical[4] == projected[4],
                "投影代次只描述请求降质，当前用户与权威历史不被吸收",
            ),
            check(
                "crossTier.nextSafeTier3IsDurableState",
                Tier3RecoveryCommit::RequireAfterEmergency
                    != Tier3RecoveryCommit::ClearAfterSummary,
                "紧急恢复的后续摘要要求与摘要成功清除是两个显式提交动作",
            ),
        ],
    ))
}
