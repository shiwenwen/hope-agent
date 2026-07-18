use super::{EvalArtifactStore, EvalIntegrity, EvalLocalExportResult, EvalRepository};
use anyhow::{anyhow, bail, Context, Result};
use ha_eval_spec::model::{
    reject_embedded_secrets, validate_evidence_shape, ModelCampaignEvidence, ModelCampaignSource,
};
use serde::Serialize;
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;

const LOCAL_BUNDLE_SCHEMA_VERSION: &str = "eval-local-bundle.v1";
const MAX_EVIDENCE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXPORT_BYTES: usize = 512 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalBundleManifest<'a> {
    schema_version: &'static str,
    experiment_id: &'a str,
    source: ModelCampaignSource,
    integrity: EvalIntegrity,
    reference: &'a str,
    dirty: bool,
    app_version: &'a str,
    exported_at: String,
    signed: bool,
    release_eligible: bool,
    notice: &'static str,
    campaigns: Vec<LocalBundleCampaign>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalBundleCampaign {
    campaign_id: String,
    path: String,
    sha256: String,
}

/// Export one local diagnostic experiment for offline diagnosis. This format
/// is intentionally unsigned and cannot be consumed by the protected bundle
/// importer. Only validated evidence JSON is included; trace, request,
/// runtime config and provider credentials never enter the archive.
pub fn export_local_evidence_bundle(
    experiment_id: &str,
    output_path: &Path,
    repository: &EvalRepository,
    artifacts: &EvalArtifactStore,
) -> Result<EvalLocalExportResult> {
    let experiment = repository
        .get_experiment(experiment_id)?
        .ok_or_else(|| anyhow!("evaluation experiment not found"))?;
    if experiment.source != ModelCampaignSource::LocalApp
        || experiment.integrity != EvalIntegrity::LocalDiagnostic
    {
        bail!("only local App diagnostic evidence can be exported as a local bundle");
    }
    validate_output_path(output_path)?;

    let artifact_refs = repository.campaign_evidence_artifacts(experiment_id)?;
    if artifact_refs.is_empty() {
        bail!("evaluation experiment has no completed evidence to export");
    }
    let mut evidence_files = Vec::with_capacity(artifact_refs.len());
    let mut manifest_campaigns = Vec::with_capacity(artifact_refs.len());
    let mut expanded_bytes = 0usize;
    for (index, (campaign_id, sha256)) in artifact_refs.into_iter().enumerate() {
        let bytes = artifacts.read(&sha256, MAX_EVIDENCE_BYTES)?;
        expanded_bytes = expanded_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| anyhow!("local evaluation export size overflow"))?;
        if expanded_bytes > MAX_EXPORT_BYTES {
            bail!("local evaluation export exceeds 512 MiB");
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("decoding evidence for campaign {campaign_id}"))?;
        reject_embedded_secrets(&value, "$.evidence")?;
        let evidence: ModelCampaignEvidence = serde_json::from_value(value)?;
        validate_evidence_shape(&evidence)?;
        if evidence.source != ModelCampaignSource::LocalApp || evidence.campaign_id != campaign_id {
            bail!("local evaluation evidence identity does not match its database record");
        }
        let path = format!("campaigns/{index:04}/evidence.json");
        manifest_campaigns.push(LocalBundleCampaign {
            campaign_id,
            path: path.clone(),
            sha256,
        });
        evidence_files.push((path, bytes));
    }

    let manifest = LocalBundleManifest {
        schema_version: LOCAL_BUNDLE_SCHEMA_VERSION,
        experiment_id,
        source: ModelCampaignSource::LocalApp,
        integrity: EvalIntegrity::LocalDiagnostic,
        reference: &experiment.reference,
        dirty: experiment.dirty,
        app_version: &experiment.app_version,
        exported_at: chrono::Utc::now().to_rfc3339(),
        signed: false,
        release_eligible: false,
        notice:
            "Local diagnostic only. This archive is unsigned and cannot become release evidence.",
        campaigns: manifest_campaigns,
    };
    let manifest_value = serde_json::to_value(&manifest)?;
    reject_embedded_secrets(&manifest_value, "$.manifest")?;
    let manifest_bytes = ha_eval_spec::canonical_json(&manifest_value)?;

    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    writer.start_file("manifest.json", options)?;
    writer.write_all(&manifest_bytes)?;
    for (path, bytes) in evidence_files {
        writer.start_file(path, options)?;
        writer.write_all(&bytes)?;
    }
    let bytes = writer.finish()?.into_inner();
    if bytes.len() > MAX_EXPORT_BYTES {
        bail!("compressed local evaluation export exceeds 512 MiB");
    }
    crate::platform::write_atomic(output_path, &bytes)
        .with_context(|| format!("writing local evaluation bundle {}", output_path.display()))?;
    Ok(EvalLocalExportResult {
        experiment_id: experiment_id.to_string(),
        output_path: output_path.to_string_lossy().into_owned(),
        bundle_sha256: super::artifact_sha256(&bytes),
        campaign_count: u32::try_from(manifest.campaigns.len())?,
        signed: false,
        release_eligible: false,
    })
}

fn validate_output_path(path: &Path) -> Result<()> {
    if path.file_name().is_none() {
        bail!("local evaluation export path must name a file");
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| anyhow!("local evaluation export path must have a parent directory"))?;
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        bail!("local evaluation export parent must be a regular directory");
    }
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("local evaluation export destination must be a regular file");
        }
    }
    Ok(())
}
