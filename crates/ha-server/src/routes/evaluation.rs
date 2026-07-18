use axum::extract::Path;
use axum::Json;
use ha_core::blocking::run_blocking;
use ha_core::evaluation::EvalArtifactStore;
use ha_core::evaluation::ModelCampaignTier;
use ha_core::evaluation::{
    CodingHistorySource, DomainHistorySource, EvalAnnotationRecord, EvalBaselineRecord,
    EvalCatalog, EvalCompareQuery, EvalCompareResult, EvalExperimentDetail, EvalExperimentRecord,
    EvalHistoryQuery, EvalHistorySource, EvalQueryService, EvalReadiness, EvalRepository,
    EvalTrendPoint, EvalTrendQuery, EvalTrialDetail,
};

use crate::error::AppError;
use crate::routes::helpers::session_db;

fn reconcile_import_trust(repository: &EvalRepository) -> anyhow::Result<()> {
    let path = std::env::current_exe()
        .ok()
        .into_iter()
        .flat_map(|path| {
            path.ancestors()
                .map(std::path::Path::to_path_buf)
                .collect::<Vec<_>>()
        })
        .flat_map(|ancestor| [ancestor.join("Resources"), ancestor])
        .chain(std::env::current_dir().ok().into_iter().flat_map(|path| {
            path.ancestors()
                .map(std::path::Path::to_path_buf)
                .collect::<Vec<_>>()
        }))
        .map(|root| root.join("evals/live/trust/evidence-keys.json"))
        .find(|candidate| candidate.is_file());
    match path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("evidence trust registry is unavailable"))
        .and_then(ha_core::evaluation::load_evidence_trust_registry_file)
    {
        Ok(trust) => {
            repository.refresh_import_signature_status(&trust)?;
        }
        Err(_) => {
            repository.mark_import_signature_keys_missing()?;
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct EvalHistoryBody {
    pub query: EvalHistoryQuery,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCompareBody {
    pub query: EvalCompareQuery,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalTrendBody {
    pub query: EvalTrendQuery,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalBaselineCreateBody {
    pub experiment_id: String,
    pub tier: ModelCampaignTier,
    pub note: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalBaselineListBody {
    pub tier: Option<ModelCampaignTier>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalAnnotationCreateBody {
    pub experiment_id: String,
    pub campaign_id: Option<String>,
    pub trial_id: Option<String>,
    pub text: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalPinBody {
    pub experiment_id: String,
    pub pinned: bool,
}

pub async fn readiness() -> Json<EvalReadiness> {
    Json(EvalReadiness {
        available: true,
        can_run: false,
        remote_run_enabled: false,
        signed_import_available: false,
        hello: None,
        issues: vec!["remote_evaluation_runtime_disabled".to_string()],
        signed_import_issues: vec!["signed_import_requires_desktop_owner_api".to_string()],
    })
}

pub async fn catalog() -> Result<Json<EvalCatalog>, AppError> {
    Ok(Json(EvalCatalog {
        readiness: readiness().await.0,
        profiles: Vec::new(),
        suites: Vec::new(),
        models: ha_core::evaluation::list_model_options()?,
    }))
}

pub async fn model_options() -> Result<Json<Vec<ha_core::evaluation::EvalModelOption>>, AppError> {
    Ok(Json(ha_core::evaluation::list_model_options()?))
}

pub async fn history(
    Json(body): Json<EvalHistoryBody>,
) -> Result<Json<Vec<EvalExperimentRecord>>, AppError> {
    let query = body.query;
    let limit = query.limit.clamp(1, 200);
    let mut repository_query = query.clone();
    repository_query.limit = limit.saturating_add(query.offset).min(200);
    repository_query.offset = 0;
    let legacy_query = repository_query.clone();
    let mut records = run_blocking(move || {
        let repository = EvalRepository::default_repository()?;
        reconcile_import_trust(&repository)?;
        repository.list_experiments(&repository_query)
    })
    .await?;
    let mut legacy = session_db()?
        .clone()
        .run(move |db| {
            let mut records = CodingHistorySource::new(db).list(&legacy_query)?;
            records.extend(DomainHistorySource::new(db).list(&legacy_query)?);
            Ok::<_, anyhow::Error>(records)
        })
        .await?;
    records.append(&mut legacy);
    records.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(Json(
        records
            .into_iter()
            .skip(query.offset as usize)
            .take(limit as usize)
            .collect(),
    ))
}

pub async fn experiment(
    Path(experiment_id): Path<String>,
) -> Result<Json<EvalExperimentDetail>, AppError> {
    let repository_id = experiment_id.clone();
    if let Some(detail) = run_blocking(move || {
        let repository = EvalRepository::default_repository()?;
        reconcile_import_trust(&repository)?;
        repository.detail(&repository_id)
    })
    .await?
    {
        return Ok(Json(detail));
    }
    let detail = if let Some(id) = experiment_id.strip_prefix("coding:").map(str::to_string) {
        session_db()?
            .clone()
            .run(move |db| {
                Ok::<_, anyhow::Error>(
                    db.get_coding_benchmark_campaign(&id)?
                        .map(|campaign| ha_core::evaluation::coding_detail(&campaign)),
                )
            })
            .await?
    } else if let Some(id) = experiment_id.strip_prefix("domain:").map(str::to_string) {
        session_db()?
            .clone()
            .run(move |db| {
                Ok::<_, anyhow::Error>(
                    db.get_domain_eval_campaign(&id)?
                        .map(|campaign| ha_core::evaluation::domain_detail(&campaign)),
                )
            })
            .await?
    } else {
        None
    }
    .ok_or_else(|| AppError::not_found("evaluation experiment not found"))?;
    Ok(Json(detail))
}

pub async fn remote_run_disabled() -> Result<Json<serde_json::Value>, AppError> {
    Err(AppError::forbidden(
        "Remote real-model evaluation is disabled; use the signed desktop Sidecar or a protected Runner",
    ))
}

pub async fn compare(
    Json(body): Json<EvalCompareBody>,
) -> Result<Json<EvalCompareResult>, AppError> {
    Ok(Json(
        run_blocking(move || {
            let repository = EvalRepository::default_repository()?;
            reconcile_import_trust(&repository)?;
            EvalQueryService::new(repository, EvalArtifactStore::default_store()?)
                .compare(&body.query)
        })
        .await?,
    ))
}

pub async fn trends(
    Json(body): Json<EvalTrendBody>,
) -> Result<Json<Vec<EvalTrendPoint>>, AppError> {
    Ok(Json(
        run_blocking(move || {
            let repository = EvalRepository::default_repository()?;
            reconcile_import_trust(&repository)?;
            EvalQueryService::new(repository, EvalArtifactStore::default_store()?)
                .trends(&body.query)
        })
        .await?,
    ))
}

pub async fn trial(
    Path((experiment_id, campaign_id, trial_id)): Path<(String, String, String)>,
) -> Result<Json<EvalTrialDetail>, AppError> {
    Ok(Json(
        run_blocking(move || {
            EvalQueryService::new(
                EvalRepository::default_repository()?,
                EvalArtifactStore::default_store()?,
            )
            .trial(&experiment_id, &campaign_id, &trial_id)
        })
        .await?,
    ))
}

pub async fn set_pinned(
    Json(body): Json<EvalPinBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    run_blocking(move || {
        EvalRepository::default_repository()?
            .set_experiment_pinned(&body.experiment_id, body.pinned)
    })
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn create_baseline(
    Json(body): Json<EvalBaselineCreateBody>,
) -> Result<Json<EvalBaselineRecord>, AppError> {
    Ok(Json(
        run_blocking(move || {
            EvalRepository::default_repository()?.create_baseline(
                &body.experiment_id,
                body.tier,
                "remote_api_owner",
                body.note.as_deref(),
            )
        })
        .await?,
    ))
}

pub async fn list_baselines(
    Json(body): Json<EvalBaselineListBody>,
) -> Result<Json<Vec<EvalBaselineRecord>>, AppError> {
    Ok(Json(
        run_blocking(move || {
            let repository = EvalRepository::default_repository()?;
            reconcile_import_trust(&repository)?;
            repository.list_baselines(body.tier)
        })
        .await?,
    ))
}

pub async fn delete_baseline(Path(baseline_id): Path<String>) -> Result<Json<bool>, AppError> {
    Ok(Json(
        run_blocking(move || EvalRepository::default_repository()?.delete_baseline(&baseline_id))
            .await?,
    ))
}

pub async fn create_annotation(
    Json(body): Json<EvalAnnotationCreateBody>,
) -> Result<Json<EvalAnnotationRecord>, AppError> {
    Ok(Json(
        run_blocking(move || {
            EvalRepository::default_repository()?.create_annotation(
                &body.experiment_id,
                body.campaign_id.as_deref(),
                body.trial_id.as_deref(),
                &body.text,
            )
        })
        .await?,
    ))
}

pub async fn list_annotations(
    Path(experiment_id): Path<String>,
) -> Result<Json<Vec<EvalAnnotationRecord>>, AppError> {
    Ok(Json(
        run_blocking(move || {
            EvalRepository::default_repository()?.list_annotations(&experiment_id)
        })
        .await?,
    ))
}
