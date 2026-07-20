use anyhow::{anyhow, bail, Context, Result};
use ha_core::coding_eval::{self, CodingEvalFixture, GoldTaskPackRunInput};
use ha_core::domain_eval::{self, RunDomainEvalFixtureInput};
use ha_core::memory::{claims, dreaming, SqliteMemoryBackend};
use ha_core::session::SessionDB;
use ha_eval_spec::{
    read_json, resolve_contained, CaseResult, EvalAdapter, EvalCheck, EvalStatus, PlannedCase,
    PlannedSuite,
};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

pub async fn run_case(
    root: &Path,
    suite: &PlannedSuite,
    case: &PlannedCase,
    attempt: u8,
) -> Result<CaseResult> {
    if std::env::var("HA_EVAL_NETWORK").as_deref() != Ok("deny") {
        bail!("deterministic adapter requires network deny policy");
    }
    let temp = tempfile::tempdir().context("creating isolated adapter directory")?;
    std::env::set_var("HA_DATA_DIR", temp.path().join("runtime"));
    let mut outcome = match suite.adapter {
        EvalAdapter::CodingFixturePatch => run_coding_fixture(root, suite, case).await?,
        EvalAdapter::CodingGoldFixturePatch => run_coding_gold(temp.path(), suite, case).await?,
        EvalAdapter::DomainTraceFixture => run_domain(temp.path(), suite, case).await?,
        EvalAdapter::DreamingGolden => run_dreaming(root, temp.path(), suite, case)?,
        EvalAdapter::MemoryRetrievalScale => run_memory_retrieval(suite, case)?,
    };
    outcome.attempt = attempt;
    Ok(outcome)
}

async fn run_coding_fixture(
    root: &Path,
    suite: &PlannedSuite,
    case: &PlannedCase,
) -> Result<CaseResult> {
    let path = case_asset(root, suite, case)?;
    let value: Value = read_json(&path)?;
    reject_model_configuration(&value, "$")?;
    let fixture: CodingEvalFixture = serde_json::from_value(value)?;
    let db = runtime_eval_db()?;
    let report = coding_eval::evaluate(db, &fixture).await?;
    let checks = report
        .outcomes
        .iter()
        .map(|outcome| EvalCheck {
            name: outcome.name.clone(),
            status: if outcome.passed {
                EvalStatus::Passed
            } else {
                EvalStatus::Failed
            },
            detail: outcome.detail.clone(),
            metric: None,
            advisory: false,
        })
        .collect();
    Ok(base_result(
        suite,
        case,
        if report.passed() {
            EvalStatus::Passed
        } else {
            EvalStatus::Failed
        },
        checks,
        None,
    ))
}

async fn run_coding_gold(
    temp: &Path,
    suite: &PlannedSuite,
    case: &PlannedCase,
) -> Result<CaseResult> {
    let _ = temp;
    let db = runtime_eval_db()?;
    let report = coding_eval::run_gold_task_pack(
        db,
        GoldTaskPackRunInput {
            ids: vec![case.id.clone()],
            execution_mode: Some("fixture_patch".to_string()),
            record_eval_runs: false,
            record_pack_run: false,
            evaluate_goal: true,
            providers: Vec::new(),
            model_chain: Vec::new(),
            ..Default::default()
        },
    )
    .await?;
    if report.selected_cases != 1 || report.automated_cases != 1 {
        bail!("gold case {} is missing or not automated", case.id);
    }
    let case_report = report
        .cases
        .first()
        .ok_or_else(|| anyhow!("gold runner returned no case"))?;
    let checks = case_report
        .report
        .as_ref()
        .map(|fixture| {
            fixture
                .outcomes
                .iter()
                .map(|outcome| EvalCheck {
                    name: outcome.name.clone(),
                    status: if outcome.passed {
                        EvalStatus::Passed
                    } else {
                        EvalStatus::Failed
                    },
                    detail: outcome.detail.clone(),
                    metric: None,
                    advisory: false,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(base_result(
        suite,
        case,
        if report.passed {
            EvalStatus::Passed
        } else {
            EvalStatus::Failed
        },
        checks,
        case_report.error.clone(),
    ))
}

async fn run_domain(temp: &Path, suite: &PlannedSuite, case: &PlannedCase) -> Result<CaseResult> {
    let _ = temp;
    let db = runtime_eval_db()?;
    let fixture = domain_eval::deterministic_domain_eval_fixture(
        db.as_ref(),
        &case.id,
        &format!("release-eval:{}", case.id),
    )?;
    if fixture.execution_mode != "trace_fixture"
        || !fixture.execution.providers.is_empty()
        || !fixture.execution.model_chain.is_empty()
    {
        bail!("domain deterministic fixture unexpectedly contains agent/provider configuration");
    }
    let report =
        SessionDB::run_domain_eval_fixture(db, RunDomainEvalFixtureInput { fixture }).await?;
    let mut checks = report
        .checks
        .iter()
        .map(|check| EvalCheck {
            name: check.name.clone(),
            status: if check.status == "passed" {
                EvalStatus::Passed
            } else {
                EvalStatus::Failed
            },
            detail: format!(
                "{}; expected={}, actual={}",
                check.detail, check.expected, check.actual
            ),
            metric: None,
            advisory: false,
        })
        .collect::<Vec<_>>();
    if let Some(eval_run) = &report.eval_run {
        checks.extend(eval_run.report.checks.iter().map(|check| EvalCheck {
            name: format!("scorer.{}", check.name),
            status: if check.status == "passed" {
                EvalStatus::Passed
            } else {
                EvalStatus::Failed
            },
            detail: format!(
                "{}; expected={}, actual={}",
                check.detail, check.expected, check.actual
            ),
            metric: Some(check.score),
            advisory: false,
        }));
        if let Some(quality_checks) = eval_run
            .report
            .quality
            .get("checks")
            .and_then(Value::as_array)
        {
            checks.extend(quality_checks.iter().map(|check| {
                let status = check
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("failed");
                EvalCheck {
                    name: format!(
                        "quality.{}",
                        check
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                    ),
                    status: if matches!(status, "passed" | "advisory") {
                        EvalStatus::Passed
                    } else {
                        EvalStatus::Failed
                    },
                    detail: check
                        .get("body")
                        .and_then(Value::as_str)
                        .unwrap_or("quality check has no detail")
                        .to_string(),
                    metric: None,
                    advisory: status == "advisory",
                }
            }));
        }
    }
    Ok(base_result(
        suite,
        case,
        if report.passed {
            EvalStatus::Passed
        } else {
            EvalStatus::Failed
        },
        checks,
        report.error,
    ))
}

fn run_dreaming(
    root: &Path,
    temp: &Path,
    suite: &PlannedSuite,
    case: &PlannedCase,
) -> Result<CaseResult> {
    let path = case_asset(root, suite, case)?;
    let value: Value = read_json(&path)?;
    reject_model_configuration(&value, "$")?;
    let fixture: dreaming::eval::DreamingFixture = serde_json::from_value(value)?;
    let backend = Arc::new(SqliteMemoryBackend::open(&temp.join("memory.db"))?);
    claims::init_claim_store(backend.clone());
    let report = dreaming::eval::evaluate(backend.as_ref(), &fixture)?;
    let checks = report
        .outcomes
        .iter()
        .map(|outcome| EvalCheck {
            name: outcome.name.clone(),
            status: if outcome.passed {
                EvalStatus::Passed
            } else {
                EvalStatus::Failed
            },
            detail: outcome.detail.clone(),
            metric: None,
            advisory: false,
        })
        .collect();
    Ok(base_result(
        suite,
        case,
        if report.passed() {
            EvalStatus::Passed
        } else {
            EvalStatus::Failed
        },
        checks,
        None,
    ))
}

fn run_memory_retrieval(suite: &PlannedSuite, case: &PlannedCase) -> Result<CaseResult> {
    if case.id == "source-fusion-scale" {
        let report = ha_core::agent::run_source_fusion_scale_eval();
        let checks = vec![
            EvalCheck {
                name: "sourceFusion.boundedUniqueSelection".to_string(),
                status: if report.passed {
                    EvalStatus::Passed
                } else {
                    EvalStatus::Failed
                },
                detail: format!(
                    "{} candidates -> {} selected ({} unique)",
                    report.candidates, report.selected, report.unique_selected
                ),
                metric: Some(report.selected as f64),
                advisory: false,
            },
            EvalCheck {
                name: "latency.sourceFusion".to_string(),
                status: EvalStatus::Passed,
                detail: format!("advisory latency {:.3}ms", report.elapsed_ms),
                metric: Some(report.elapsed_ms),
                advisory: true,
            },
        ];
        return Ok(base_result(
            suite,
            case,
            if report.passed {
                EvalStatus::Passed
            } else {
                EvalStatus::Failed
            },
            checks,
            (!report.failures.is_empty()).then(|| report.failures.join("; ")),
        ));
    }
    if case.id != "sqlite-hybrid-scale" {
        bail!("unknown memory retrieval case {}", case.id);
    }
    let report = ha_core::memory::retrieval_scale_eval::run_retrieval_scale_eval();
    let mut checks = report
        .quality
        .iter()
        .map(|(name, metric)| EvalCheck {
            name: name.clone(),
            status: if report
                .failures
                .iter()
                .any(|failure| failure.starts_with(name))
            {
                EvalStatus::Failed
            } else {
                EvalStatus::Passed
            },
            detail: format!("quality metric {name}={metric:.4}"),
            metric: Some(*metric),
            advisory: false,
        })
        .collect::<Vec<_>>();
    checks.extend(report.latency_ms.iter().map(|(name, metric)| EvalCheck {
        name: format!("latency.{name}"),
        status: EvalStatus::Passed,
        detail: format!("advisory latency {metric:.3}ms"),
        metric: Some(*metric),
        advisory: true,
    }));
    Ok(base_result(
        suite,
        case,
        if report.passed {
            EvalStatus::Passed
        } else {
            EvalStatus::Failed
        },
        checks,
        (!report.failures.is_empty()).then(|| report.failures.join("; ")),
    ))
}

fn case_asset(root: &Path, suite: &PlannedSuite, case: &PlannedCase) -> Result<std::path::PathBuf> {
    let relative = case
        .path
        .as_deref()
        .ok_or_else(|| anyhow!("suite {} case {} has no asset path", suite.id, case.id))?;
    resolve_contained(&root.join("evals/suites").join(&suite.id), relative)
}

fn base_result(
    suite: &PlannedSuite,
    case: &PlannedCase,
    status: EvalStatus,
    checks: Vec<EvalCheck>,
    error: Option<String>,
) -> CaseResult {
    CaseResult {
        suite_id: suite.id.clone(),
        case_id: case.id.clone(),
        case_digest: case.digest.clone(),
        status,
        duration_ms: 0,
        attempt: 1,
        checks,
        error,
    }
}

fn runtime_eval_db() -> Result<Arc<SessionDB>> {
    ha_core::init_runtime("eval");
    ha_core::get_session_db()
        .cloned()
        .ok_or_else(|| anyhow!("eval runtime did not initialize SessionDB"))
}

fn reject_model_configuration(value: &Value, location: &str) -> Result<()> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
                if matches!(
                    normalized.as_str(),
                    "provider"
                        | "providers"
                        | "providerid"
                        | "providertype"
                        | "providerconfig"
                        | "providerconfigs"
                        | "activeprovider"
                        | "fallbackproviders"
                        | "model"
                        | "models"
                        | "modelid"
                        | "modelchain"
                        | "modeloverride"
                        | "modelconfig"
                        | "modelconfigs"
                        | "functionmodels"
                        | "functionmodelsconfig"
                        | "activemodel"
                        | "fallbackmodels"
                        | "apikey"
                        | "apitoken"
                        | "accesstoken"
                        | "accesskey"
                        | "secretkey"
                        | "credentials"
                        | "authorization"
                        | "baseurl"
                        | "endpoint"
                ) && !child.is_null()
                    && child.as_array().is_none_or(|items| !items.is_empty())
                    && child.as_str().is_none_or(|text| !text.is_empty())
                {
                    bail!("deterministic eval rejects model/provider field at {location}.{key}");
                }
                if matches!(normalized.as_str(), "executionmode" | "mode")
                    && child.as_str().is_some_and(|mode| {
                        let mode = mode.trim().to_ascii_lowercase().replace(['-', ' '], "_");
                        matches!(
                            mode.as_str(),
                            "agent"
                                | "external_model"
                                | "real_model"
                                | "live_model"
                                | "model"
                                | "llm"
                                | "provider"
                                | "mock_provider"
                        )
                    })
                {
                    bail!("deterministic eval rejects agent execution at {location}.{key}");
                }
                reject_model_configuration(child, &format!("{location}.{key}"))?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                reject_model_configuration(child, &format!("{location}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_fixtures_reject_all_model_configuration_shapes() {
        for fixture in [
            serde_json::json!({"model": "gpt-test"}),
            serde_json::json!({"modelId": "gpt-test"}),
            serde_json::json!({"providerId": "provider-test"}),
            serde_json::json!({"providerConfig": {"id": "provider-test"}}),
            serde_json::json!({"functionModels": {"automation": "gpt-test"}}),
            serde_json::json!({"baseUrl": "https://example.invalid"}),
            serde_json::json!({"nested": {"credentials": {"token": "secret"}}}),
            serde_json::json!({"executionMode": "external-model"}),
            serde_json::json!({"executionMode": "real model"}),
        ] {
            assert!(reject_model_configuration(&fixture, "$").is_err());
        }
    }

    #[test]
    fn deterministic_fixture_without_model_configuration_is_allowed() {
        let fixture = serde_json::json!({
            "name": "fixture",
            "executionMode": "fixture_patch",
            "checks": {"expectedStatus": "passed"}
        });
        reject_model_configuration(&fixture, "$").unwrap();
    }
}
