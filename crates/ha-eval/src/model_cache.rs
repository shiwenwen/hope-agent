//! Cache diagnostics over production model-call receipts, never a Provider loop.
//! Phase markers contain only sequence boundaries, counts and digests. Missing
//! receipts stay unknown; the ordinary eval budget estimate is not a cache bill.

use anyhow::{anyhow, bail, Context, Result};
use ha_eval_spec::model::{
    validate_model_trace, AttributionCompleteness, ModelCampaignEvent, ModelCampaignEvidence,
    ModelCampaignOutcome, ModelShardResult, ModelTrialResult, TraceSummary,
    EVIDENCE_SCHEMA_VERSION, SHARD_SCHEMA_VERSION, TRIAL_SCHEMA_VERSION,
};
use ha_eval_spec::{digest_file, read_json, sha256_bytes, write_json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) const PHASE_EVENT: &str = "benchmark.cache_phase";

pub(crate) struct PhaseBoundary {
    pub name: String,
    pub after_seq: u64,
    pub through_seq: u64,
    pub wall_ms: u64,
    pub quality_passed: Option<bool>,
}

pub(crate) fn append_phase_events(
    trace: &mut TraceSummary,
    events: &mut Vec<ModelCampaignEvent>,
    phases: &[PhaseBoundary],
    namespace_digest: &str,
    synthetic: bool,
) {
    let timestamp_ms = events.last().map_or(0, |event| event.timestamp_ms);
    for phase in phases {
        let mut attributes = BTreeMap::from([
            ("afterSeq".into(), json!(phase.after_seq)),
            ("throughSeq".into(), json!(phase.through_seq)),
            ("wallMs".into(), json!(phase.wall_ms)),
            ("namespaceDigest".into(), json!(namespace_digest)),
            ("synthetic".into(), json!(synthetic)),
        ]);
        if let Some(passed) = phase.quality_passed {
            attributes.insert("qualityPassed".into(), json!(passed));
        }
        events.push(ModelCampaignEvent {
            seq: events.last().map_or(1, |event| event.seq + 1),
            event: PHASE_EVENT.into(),
            timestamp_ms,
            span_id: format!(
                "span_{}",
                &sha256_bytes(format!("{}:cache:{}", trace.trace_id, phase.name).as_bytes())[..24]
            ),
            parent_span_id: trace.root_span_id.clone(),
            key: Some(phase.name.clone()),
            status: "completed".into(),
            // Harness observation must not inflate the product critical path.
            duration_ms: 0,
            attributes,
        });
        trace.span_count += 1;
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pricing {
    schema_version: String,
    models: Vec<CachePrice>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CachePrice {
    provider_id: String,
    model_id: String,
    /// Explicit user-supplied assumptions, including the applicable write TTL.
    basis: String,
    input_usd_per_million: f64,
    output_usd_per_million: f64,
    cache_read_usd_per_million: f64,
    cache_write_usd_per_million: f64,
}

impl Pricing {
    fn validate(&self) -> Result<()> {
        if self.schema_version != "cache-benchmark-pricing.v1" || self.models.is_empty() {
            bail!("invalid cache pricing schema or empty model list");
        }
        let mut keys = BTreeSet::new();
        for price in &self.models {
            if price.provider_id.is_empty()
                || price.model_id.is_empty()
                || price.basis.trim().is_empty()
                || price.basis.len() > 256
                || price.basis.contains(['\r', '\n'])
                || !keys.insert((&price.provider_id, &price.model_id))
                || [
                    price.input_usd_per_million,
                    price.output_usd_per_million,
                    price.cache_read_usd_per_million,
                    price.cache_write_usd_per_million,
                ]
                .iter()
                .any(|price| !price.is_finite() || *price < 0.0)
            {
                bail!("cache pricing requires unique model identities, a basis and finite nonnegative prices");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Distribution {
    samples: usize,
    p50: Option<f64>,
    p95: Option<f64>,
}

fn distribution(values: impl IntoIterator<Item = f64>) -> Distribution {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let percentile = |fraction: f64| {
        (!values.is_empty()).then(|| values[(values.len() as f64 * fraction).ceil() as usize - 1])
    };
    Distribution {
        samples: values.len(),
        p50: percentile(0.5),
        p95: percentile(0.95),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct SeriesIdentity {
    commit_sha: String,
    plan_digest: String,
    model_digest: String,
    scenario_digest: String,
    runtime_config_digest: Option<String>,
    arm: String,
    phase: String,
    evidence_mode: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PhaseResult {
    trial_id: String,
    attempt: u8,
    identity: SeriesIdentity,
    trial_outcome: ModelCampaignOutcome,
    quality_passed: Option<bool>,
    eligible_for_comparison: bool,
    model_calls: usize,
    failed_calls: usize,
    operations: BTreeMap<String, usize>,
    wall_ms: u64,
    model_active_ms: u64,
    call_duration_ms: Distribution,
    ttft_ms: Distribution,
    #[serde(skip)]
    ttft_samples_ms: Vec<f64>,
    context_input_tokens: Option<u64>,
    fresh_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
    cache_read_ratio: Option<f64>,
    modeled_cost_usd: Option<f64>,
    uncached_counterfactual_usd: Option<f64>,
    modeled_savings_usd: Option<f64>,
    summary_request_modes: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Series {
    identity: SeriesIdentity,
    observed_attempts: usize,
    eligible_attempts: usize,
    wall_ms: Distribution,
    ttft_ms: Distribution,
    cache_read_ratio: Distribution,
    modeled_cost_usd: Distribution,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: &'static str,
    input_digests: Vec<String>,
    pricing_snapshot_digest: Option<String>,
    pricing_bases: Vec<String>,
    trials: usize,
    attempts: usize,
    unassigned_model_calls: usize,
    phases: Vec<PhaseResult>,
    series: Vec<Series>,
    notes: Vec<&'static str>,
}

fn exact_sum(calls: &[&ModelCampaignEvent], key: &str) -> Option<u64> {
    if calls.is_empty() {
        return None;
    }
    calls.iter().try_fold(0u64, |total, call| {
        total.checked_add(call.attributes.get(key)?.as_u64()?)
    })
}

fn call_cost(call: &ModelCampaignEvent, pricing: &Pricing) -> Option<(f64, f64)> {
    let attr = &call.attributes;
    let price = pricing.models.iter().find(|price| {
        attr.get("providerId").and_then(Value::as_str) == Some(price.provider_id.as_str())
            && attr.get("modelId").and_then(Value::as_str) == Some(price.model_id.as_str())
    })?;
    let context = attr.get("contextInputTokens")?.as_u64()?;
    let read = attr.get("cacheReadTokens")?.as_u64()?;
    let output = attr.get("outputTokens")?.as_u64()?;
    let fresh = context.checked_sub(read)?;
    // When writes have the ordinary input price (e.g. implicit DeepSeek
    // caching), their unknown count cancels out of the cost equation. Keep
    // the receipt unknown; never manufacture a zero cache-write observation.
    let write_adjustment = if price.cache_write_usd_per_million == price.input_usd_per_million {
        0.0
    } else {
        let write = attr.get("cacheWriteTokens")?.as_u64()?;
        fresh.checked_sub(write)?;
        write as f64 * (price.cache_write_usd_per_million - price.input_usd_per_million)
    };
    let modeled = (fresh as f64 * price.input_usd_per_million
        + write_adjustment
        + read as f64 * price.cache_read_usd_per_million
        + output as f64 * price.output_usd_per_million)
        / 1_000_000.0;
    let counterfactual = (context as f64 * price.input_usd_per_million
        + output as f64 * price.output_usd_per_million)
        / 1_000_000.0;
    (modeled.is_finite() && counterfactual.is_finite()).then_some((modeled, counterfactual))
}

fn attempt_phases(
    trial: &ModelTrialResult,
    attempt: u8,
    outcome: ModelCampaignOutcome,
    attribution: AttributionCompleteness,
    trace: &TraceSummary,
    events: &[ModelCampaignEvent],
    pricing: Option<&Pricing>,
) -> Result<(Vec<PhaseResult>, usize)> {
    validate_model_trace(trace, events, "cache benchmark input")?;
    let all_calls = events
        .iter()
        .filter(|event| event.event == "model.call")
        .collect::<Vec<_>>();
    let mut assigned = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut after_previous = 0;
    let mut isolation = None;
    let mut results = Vec::new();
    for marker in events.iter().filter(|event| event.event == PHASE_EVENT) {
        let phase = marker
            .key
            .as_deref()
            .ok_or_else(|| anyhow!("cache phase has no name"))?;
        if phase.is_empty()
            || phase.len() > 48
            || !phase
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            bail!("invalid cache phase name");
        }
        let number = |key: &str| {
            marker
                .attributes
                .get(key)
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("cache phase {phase} is missing {key}"))
        };
        let after = number("afterSeq")?;
        let through = number("throughSeq")?;
        if after != after_previous
            || through <= after
            || through >= marker.seq
            || !names.insert(phase)
        {
            bail!("cache phase boundaries must be unique, contiguous and ordered");
        }
        after_previous = through;
        let namespace = marker
            .attributes
            .get("namespaceDigest")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("cache phase has no isolation namespace digest"))?;
        if namespace.len() != 64 || !namespace.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("invalid cache isolation digest");
        }
        let synthetic = marker
            .attributes
            .get("synthetic")
            .and_then(Value::as_bool)
            .ok_or_else(|| anyhow!("cache phase has no evidence mode"))?;
        if isolation
            .replace((namespace, synthetic))
            .is_some_and(|previous| previous != (namespace, synthetic))
        {
            bail!("cache phases changed namespace or evidence mode within an attempt");
        }
        let calls = all_calls
            .iter()
            .copied()
            .filter(|call| call.seq > after && call.seq <= through)
            .collect::<Vec<_>>();
        for call in &calls {
            if !assigned.insert(call.seq) {
                bail!("cache phase double-counts a model call");
            }
        }
        let failed_calls = calls
            .iter()
            .filter(|call| call.status != "succeeded")
            .count();
        let context = exact_sum(&calls, "contextInputTokens");
        let read = exact_sum(&calls, "cacheReadTokens");
        let quality = marker
            .attributes
            .get("qualityPassed")
            .and_then(Value::as_bool);
        let costs = pricing.and_then(|pricing| {
            if calls.is_empty() {
                return None;
            }
            calls.iter().try_fold((0.0, 0.0), |(cost, counter), call| {
                let (next_cost, next_counter) = call_cost(call, pricing)?;
                Some((cost + next_cost, counter + next_counter))
            })
        });
        let ttft_samples = calls
            .iter()
            .filter_map(|call| call.attributes.get("ttftMs").and_then(Value::as_u64))
            .map(|value| value as f64)
            .collect::<Vec<_>>();
        results.push(PhaseResult {
            trial_id: trial.trial_id.clone(),
            attempt,
            identity: SeriesIdentity {
                commit_sha: trial.commit_sha.clone(),
                plan_digest: trial.plan_digest.clone(),
                model_digest: trial.model_digest.clone(),
                scenario_digest: trial.scenario_digest.clone(),
                runtime_config_digest: trial.runtime_config_digest.clone(),
                arm: trial.arm.clone(),
                phase: phase.into(),
                evidence_mode: if synthetic {
                    "synthetic_fixture"
                } else {
                    "provider_observation"
                }
                .into(),
            },
            trial_outcome: outcome,
            quality_passed: quality,
            eligible_for_comparison: outcome == ModelCampaignOutcome::Passed
                && attribution == AttributionCompleteness::Complete
                && quality != Some(false)
                && !calls.is_empty()
                && failed_calls == 0,
            model_calls: calls.len(),
            failed_calls,
            operations: calls.iter().fold(BTreeMap::new(), |mut operations, call| {
                let operation = call
                    .attributes
                    .get("operation")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown_operation");
                *operations.entry(operation.to_string()).or_default() += 1;
                operations
            }),
            wall_ms: number("wallMs")?,
            model_active_ms: calls.iter().map(|call| call.duration_ms).sum(),
            call_duration_ms: distribution(calls.iter().map(|call| call.duration_ms as f64)),
            ttft_ms: distribution(ttft_samples.iter().copied()),
            ttft_samples_ms: ttft_samples,
            context_input_tokens: context,
            fresh_input_tokens: exact_sum(&calls, "freshInputTokens"),
            output_tokens: exact_sum(&calls, "outputTokens"),
            cache_read_tokens: read,
            cache_write_tokens: exact_sum(&calls, "cacheWriteTokens"),
            cache_read_ratio: context
                .zip(read)
                .filter(|(context, read)| *context > 0 && read <= context)
                .map(|(context, read)| read as f64 / context as f64),
            modeled_cost_usd: costs.map(|(cost, _)| cost),
            uncached_counterfactual_usd: costs.map(|(_, counter)| counter),
            modeled_savings_usd: costs.map(|(cost, counter)| counter - cost),
            summary_request_modes: calls
                .iter()
                .filter_map(|call| {
                    call.attributes
                        .get("summaryRequestMode")
                        .and_then(Value::as_str)
                })
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        });
    }
    Ok((results, all_calls.len() - assigned.len()))
}

fn build_report(trials: &[ModelTrialResult], pricing: Option<&Pricing>) -> Result<Report> {
    let mut report = Report {
        schema_version: "cache-benchmark-report.v1", input_digests: Vec::new(), pricing_snapshot_digest: None,
        pricing_bases: pricing.map_or_else(Vec::new, |prices| prices.models.iter().map(|price| format!("{}::{}: {}", price.provider_id, price.model_id, price.basis)).collect()),
        trials: trials.len(), attempts: 0, unassigned_model_calls: 0, phases: Vec::new(), series: Vec::new(),
        notes: vec![
            "cold_history requests a fresh synthetic history namespace; shared system/tools may already be cached. Cache hits are observed, never assumed.",
            "Synthetic fixtures validate collection and arithmetic only; they are not measured Provider cache performance.",
            "TTFT distributions use per-call receipts, not differences of trial-wide minimum TTFT. Missing samples are excluded and sample counts are explicit.",
            "Modeled costs require an explicit USD pricing snapshot with the applicable cache-write TTL; they are not invoices. Missing usage/prices stay null.",
            "Failed quality, incomplete attribution and failed/retried attempts remain visible and are excluded from comparison distributions. Unassigned calls remain counted.",
            "P50/P95 are descriptive nearest-rank statistics. Small samples and different phase workloads do not establish speedup or statistical significance.",
        ],
    };
    for trial in trials {
        for prior in &trial.prior_attempts {
            let (phases, unassigned) = attempt_phases(
                trial,
                prior.attempt,
                prior.outcome,
                prior.attribution,
                &prior.trace,
                &prior.trace_events,
                pricing,
            )?;
            report.attempts += 1;
            report.unassigned_model_calls += unassigned;
            report.phases.extend(phases);
        }
        let (phases, unassigned) = attempt_phases(
            trial,
            trial.attempt,
            trial.outcome,
            trial.attribution,
            &trial.trace,
            &trial.trace_events,
            pricing,
        )?;
        report.attempts += 1;
        report.unassigned_model_calls += unassigned;
        report.phases.extend(phases);
    }
    if report.phases.is_empty() {
        bail!("inputs contain no cache benchmark phase receipts");
    }
    let mut groups = BTreeMap::<SeriesIdentity, Vec<&PhaseResult>>::new();
    for phase in &report.phases {
        groups
            .entry(phase.identity.clone())
            .or_default()
            .push(phase);
    }
    report.series = groups
        .into_iter()
        .map(|(identity, phases)| {
            let eligible = phases
                .iter()
                .copied()
                .filter(|phase| phase.eligible_for_comparison)
                .collect::<Vec<_>>();
            Series {
                identity,
                observed_attempts: phases.len(),
                eligible_attempts: eligible.len(),
                wall_ms: distribution(eligible.iter().map(|phase| phase.wall_ms as f64)),
                ttft_ms: distribution(
                    eligible
                        .iter()
                        .flat_map(|phase| phase.ttft_samples_ms.iter().copied()),
                ),
                cache_read_ratio: distribution(
                    eligible.iter().filter_map(|phase| phase.cache_read_ratio),
                ),
                modeled_cost_usd: distribution(
                    eligible.iter().filter_map(|phase| phase.modeled_cost_usd),
                ),
            }
        })
        .collect();
    Ok(report)
}

pub(crate) fn command_report(
    inputs: &[PathBuf],
    pricing_path: Option<&Path>,
    output: &Path,
    summary: &Path,
) -> Result<()> {
    let identity_path = |path: &Path| -> Result<PathBuf> {
        Ok(path.canonicalize().unwrap_or(std::path::absolute(path)?))
    };
    let output_path = identity_path(output)?;
    let summary_path = identity_path(summary)?;
    if output_path == summary_path
        || inputs
            .iter()
            .map(PathBuf::as_path)
            .chain(pricing_path)
            .any(|input| {
                identity_path(input).is_ok_and(|path| path == output_path || path == summary_path)
            })
    {
        bail!("cache report outputs must differ from each other and all evidence/pricing inputs");
    }
    let pricing: Option<Pricing> = pricing_path.map(read_json).transpose()?;
    if let Some(pricing) = &pricing {
        pricing.validate()?;
    }
    let mut trials = Vec::new();
    let mut identities = BTreeSet::new();
    let mut digests = Vec::new();
    for path in inputs {
        let raw: Value = read_json(path)?;
        let loaded: Vec<ModelTrialResult> = match raw.get("schemaVersion").and_then(Value::as_str) {
            Some(TRIAL_SCHEMA_VERSION) => vec![serde_json::from_value(raw)?],
            Some(SHARD_SCHEMA_VERSION) => serde_json::from_value::<ModelShardResult>(raw)?.trials,
            Some(EVIDENCE_SCHEMA_VERSION) => {
                serde_json::from_value::<ModelCampaignEvidence>(raw)?.trial_results
            }
            _ => bail!(
                "cache report expects model trial, shard or campaign evidence: {}",
                path.display()
            ),
        };
        for trial in loaded.into_iter().filter(|trial| {
            matches!(
                trial.suite_id.as_str(),
                "prompt-cache" | "prompt-cache-auto"
            )
        }) {
            if !identities.insert((trial.plan_digest.clone(), trial.trial_id.clone())) {
                bail!("duplicate cache trial input; pass a shard or its aggregate, not both");
            }
            trials.push(trial);
        }
        digests.push(digest_file(path)?);
    }
    let mut report = build_report(&trials, pricing.as_ref())?;
    report.input_digests = digests;
    report.pricing_snapshot_digest = pricing_path.map(digest_file).transpose()?;
    let mut markdown = format!("# 缓存 benchmark\n\n{} trial / {} attempt；未分配阶段的模型调用 {} 次。\n\n| 提交 / 模型 / 计划 | 阶段 | 模式 | 有效/观察 | 阶段耗时 P50/P95 ms | TTFT P50/P95 ms（样本） | 缓存读取比例 P50 | 建模费用 P50 USD |\n| --- | --- | --- | --- | --- | --- | --- | --- |\n", report.trials, report.attempts, report.unassigned_model_calls);
    let optional =
        |value: Option<f64>| value.map_or_else(|| "未知".into(), |value| format!("{value:.6}"));
    for series in &report.series {
        markdown.push_str(&format!(
            "| {:.8} / {:.8} / {:.8} | {} | {} | {}/{} | {}/{} | {}/{}（{}） | {} | {} |\n",
            series.identity.commit_sha,
            series.identity.model_digest,
            series.identity.plan_digest,
            series.identity.phase,
            series.identity.evidence_mode,
            series.eligible_attempts,
            series.observed_attempts,
            optional(series.wall_ms.p50),
            optional(series.wall_ms.p95),
            optional(series.ttft_ms.p50),
            optional(series.ttft_ms.p95),
            series.ttft_ms.samples,
            optional(series.cache_read_ratio.p50),
            optional(series.modeled_cost_usd.p50)
        ));
    }
    markdown.push_str("\nJSON 保留逐阶段用量、TTFT 样本数、摘要请求模式、失败与各版本身份；不同提交/模型/场景/配置分组统计。首次发送历史不保证全冷。合成回执仅验证链路；费用依赖显式价格和写入 TTL，均不代表账单。样本不足时不能据此判断性能提升。\n");
    write_json(output, &report)?;
    if let Some(parent) = summary.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(summary, markdown).context("writing cache benchmark summary")?;
    println!(
        "cache report: {} phases, {} attempts, {} unassigned calls",
        report.phases.len(),
        report.attempts,
        report.unassigned_model_calls
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_eval_spec::model::{
        CostMetrics, OrchestrationMetrics, TimingMetrics, TokenMetrics, ToolMetrics,
    };

    fn trial() -> ModelTrialResult {
        let mut trace = TraceSummary {
            trace_id: "trace_test".into(),
            root_span_id: "span_root".into(),
            span_count: 2,
            orphan_span_count: 0,
            closed: true,
        };
        let mut events = vec![call()];
        append_phase_events(
            &mut trace,
            &mut events,
            &[PhaseBoundary {
                name: "cold_history".into(),
                after_seq: 0,
                through_seq: 1,
                wall_ms: 120,
                quality_passed: Some(true),
            }],
            &sha256_bytes(b"namespace"),
            true,
        );
        serde_json::from_value(json!({
            "schemaVersion": TRIAL_SCHEMA_VERSION, "trialId": "mtrial_test", "campaignId": "mcampaign_test",
            "planDigest": "1".repeat(64), "commitSha": "2".repeat(40), "appVersion": "0.71.0",
            "suiteId": "prompt-cache", "suiteVersion": "1.0.0", "suiteDigest": "3".repeat(64),
            "caseId": "HA-CACHE-001", "caseDigest": "4".repeat(64), "trialIndex": 0, "seed": 1,
            "modelRole": "anchor", "arm": "control", "faultProfile": "clean", "modelDigest": "5".repeat(64),
            "scenarioDigest": "6".repeat(64), "environmentDigest": "7".repeat(64), "verifierDigest": "8".repeat(64),
            "promptDigest": "9".repeat(64), "toolSchemaDigest": "a".repeat(64), "attempt": 1,
            "startedAt": "2026-11-04T00:00:00Z", "completedAt": "2026-11-04T00:00:01Z", "outcome": "passed", "attribution": "complete",
            "timings": TimingMetrics::default(), "tokens": TokenMetrics::default(), "cost": CostMetrics::default(),
            "tools": ToolMetrics::default(), "orchestration": OrchestrationMetrics::default(), "trace": trace, "traceEvents": events
        })).unwrap()
    }

    fn call() -> ModelCampaignEvent {
        ModelCampaignEvent {
            seq: 1,
            event: "model.call".into(),
            timestamp_ms: 100,
            span_id: "span_call".into(),
            parent_span_id: "span_root".into(),
            key: None,
            status: "succeeded".into(),
            duration_ms: 90,
            attributes: BTreeMap::from([
                ("providerId".into(), json!("test")),
                ("modelId".into(), json!("model")),
                // Anthropic raw input excludes both cache buckets.
                ("inputTokens".into(), json!(100)),
                ("contextInputTokens".into(), json!(1000)),
                ("freshInputTokens".into(), json!(200)),
                ("cacheReadTokens".into(), json!(800)),
                ("cacheWriteTokens".into(), json!(100)),
                ("outputTokens".into(), json!(50)),
            ]),
        }
    }

    #[test]
    fn auto_cache_suite_is_reported_without_merging_duplicate_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("auto.json");
        let output = directory.path().join("report.json");
        let summary = directory.path().join("report.md");
        let mut auto = trial();
        auto.suite_id = "prompt-cache-auto".into();
        auto.case_id = "HA-CACHE-002".into();
        ha_eval_spec::write_json(&input, &auto).unwrap();
        command_report(std::slice::from_ref(&input), None, &output, &summary).unwrap();
        let report: Value = read_json(&output).unwrap();
        assert_eq!(report["trials"], 1);
        assert_eq!(report["phases"].as_array().unwrap().len(), 1);
        assert!(command_report(&[input.clone(), input], None, &output, &summary).is_err());
    }

    #[test]
    fn cache_cost_uses_normalized_context_and_explicit_write_price() {
        let prices: Pricing = serde_json::from_value(json!({
            "schemaVersion": "cache-benchmark-pricing.v1", "models": [{
                "providerId": "test", "modelId": "model", "basis": "synthetic 5m assumption",
                "inputUsdPerMillion": 1.0, "outputUsdPerMillion": 2.0,
                "cacheReadUsdPerMillion": 0.1, "cacheWriteUsdPerMillion": 1.25
            }]
        }))
        .unwrap();
        prices.validate().unwrap();
        let mut event = call();
        let (cost, counter) = call_cost(&event, &prices).unwrap();
        assert!((cost - 0.000405).abs() < 1e-12);
        assert!((counter - 0.0011).abs() < 1e-12);
        event.attributes.remove("cacheWriteTokens");
        assert!(call_cost(&event, &prices).is_none());
    }

    #[test]
    fn equal_write_and_input_prices_cancel_unknown_write_count_without_inventing_zero() {
        let mut prices: Pricing = serde_json::from_value(json!({
            "schemaVersion": "cache-benchmark-pricing.v1", "models": [{
                "providerId": "test", "modelId": "model", "basis": "implicit caching, writes billed as normal input",
                "inputUsdPerMillion": 0.3, "outputUsdPerMillion": 1.2,
                "cacheReadUsdPerMillion": 0.006, "cacheWriteUsdPerMillion": 0.3
            }]
        })).unwrap();
        prices.validate().unwrap();
        let mut event = call();
        event.attributes.remove("cacheWriteTokens");
        let (cost, counter) = call_cost(&event, &prices).unwrap();
        assert!((cost - 0.0001248).abs() < 1e-12);
        assert!((counter - 0.00036).abs() < 1e-12);
        assert_eq!(exact_sum(&[&event], "cacheWriteTokens"), None);
        prices.models[0].cache_write_usd_per_million = 0.4;
        assert!(call_cost(&event, &prices).is_none());
    }

    #[test]
    fn unknown_receipts_poison_sums_but_actual_zero_survives() {
        let mut event = call();
        event.attributes.insert("cacheWriteTokens".into(), json!(0));
        assert_eq!(exact_sum(&[&event], "cacheWriteTokens"), Some(0));
        let mut missing = event.clone();
        missing.attributes.remove("cacheWriteTokens");
        assert_eq!(exact_sum(&[&event, &missing], "cacheWriteTokens"), None);
        assert_eq!(exact_sum(&[], "cacheReadTokens"), None);
    }

    #[test]
    fn failures_retries_and_unknown_prices_remain_in_report() {
        let mut trial = trial();
        let mut failed = trial.as_attempt_record();
        failed.outcome = ModelCampaignOutcome::TaskFailed;
        failed.trace_events[1]
            .attributes
            .insert("qualityPassed".into(), json!(false));
        trial.attempt = 2;
        trial.prior_attempts.push(failed);
        let report = build_report(&[trial], None).unwrap();
        assert_eq!(report.trials, 1);
        assert_eq!(report.attempts, 2);
        assert_eq!(report.phases.len(), 2);
        assert!(!report.phases[0].eligible_for_comparison);
        assert!(report.phases[1].eligible_for_comparison);
        assert_eq!(report.phases[0].model_calls, 1);
        assert_eq!(report.phases[0].modeled_cost_usd, None);
        assert_eq!(report.series[0].observed_attempts, 2);
        assert_eq!(report.series[0].eligible_attempts, 1);
        assert_eq!(report.series[0].modeled_cost_usd.samples, 0);
    }

    #[test]
    fn overlapping_phase_ranges_and_duplicate_evidence_are_rejected() {
        let mut trial = trial();
        append_phase_events(
            &mut trial.trace,
            &mut trial.trace_events,
            &[PhaseBoundary {
                name: "warm_1".into(),
                after_seq: 0,
                through_seq: 1,
                wall_ms: 20,
                quality_passed: Some(true),
            }],
            &sha256_bytes(b"namespace"),
            true,
        );
        assert!(build_report(&[trial], None).is_err());
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("trial.json");
        write_json(&input, &self::trial()).unwrap();
        let output = root.path().join("report.json");
        assert!(command_report(
            &[input.clone(), input],
            None,
            &output,
            &root.path().join("report.md")
        )
        .is_err());
        assert!(!output.exists());
    }

    #[test]
    fn commits_and_configuration_are_not_pooled() {
        let first = trial();
        let mut changed = trial();
        changed.commit_sha = "b".repeat(40);
        let mut config = trial();
        config.runtime_config_digest = Some("c".repeat(64));
        let report = build_report(&[first, changed, config], None).unwrap();
        assert_eq!(report.series.len(), 3);
        assert!(report
            .series
            .iter()
            .all(|series| series.observed_attempts == 1));
    }

    #[test]
    fn phases_are_closed_payload_free_trace_leaves() {
        let mut trace = TraceSummary {
            trace_id: "trace_test".into(),
            root_span_id: "span_root".into(),
            span_count: 2,
            orphan_span_count: 0,
            closed: true,
        };
        let mut events = vec![call()];
        append_phase_events(
            &mut trace,
            &mut events,
            &[PhaseBoundary {
                name: "cold_history".into(),
                after_seq: 0,
                through_seq: 1,
                wall_ms: 120,
                quality_passed: Some(true),
            }],
            &sha256_bytes(b"namespace"),
            true,
        );
        validate_model_trace(&trace, &events, "test phase").unwrap();
        assert_eq!(events[1].duration_ms, 0);
        assert!(events[1].attributes["synthetic"].as_bool().unwrap());
        assert_eq!(events[1].attributes["qualityPassed"], true);
    }

    #[test]
    fn quantiles_expose_sample_counts_and_missing_ttft_is_not_zero() {
        assert_eq!(distribution([]).p50, None);
        let values = distribution([10.0, 50.0, 20.0]);
        assert_eq!(values.samples, 3);
        assert_eq!(values.p50, Some(20.0));
        assert_eq!(values.p95, Some(50.0));
        let event = call();
        assert_eq!(
            distribution(
                [&event]
                    .iter()
                    .filter_map(|call| call.attributes.get("ttftMs").and_then(Value::as_f64))
            )
            .samples,
            0
        );
    }
}
