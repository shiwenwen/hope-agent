use super::{EvalArtifactStore, EvalImportResult, EvalRepository};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use ha_eval_spec::app::{
    evidence_trust_key_fingerprint, validate_evidence_bundle_manifest, validate_trust_registry,
    EvidenceBundleManifest, EvidenceKeyStatus, EvidenceTrustRegistry,
};
use ha_eval_spec::canonical_json;
use ha_eval_spec::model::{validate_evidence_shape, ModelCampaignEvidence};
use ring::signature::{UnparsedPublicKey, ED25519};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const MAX_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 256;
const MANIFEST_PATH: &str = "manifest.json";
const SIGNATURE_PATH: &str = "manifest.sig";
const OFFICIAL_REPOSITORY: &str = "shiwenwen/hope-agent";
const OFFICIAL_WORKFLOW_PREFIX: &str = "shiwenwen/hope-agent/.github/workflows/model-campaign.yml@";

pub fn load_evidence_trust_registry_file(path: &Path) -> Result<EvidenceTrustRegistry> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("reading evidence trust registry {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 1024 * 1024 {
        bail!("evidence trust registry must be a regular file no larger than 1 MiB");
    }
    let trust: EvidenceTrustRegistry = serde_json::from_slice(&std::fs::read(path)?)?;
    validate_trust_registry(&trust)?;
    Ok(trust)
}

pub fn validate_evidence_trust_registry_file(path: &Path) -> Result<()> {
    load_evidence_trust_registry_file(path).map(|_| ())
}

#[derive(Debug)]
pub struct VerifiedEvidenceBundle {
    pub manifest: EvidenceBundleManifest,
    pub evidence: ModelCampaignEvidence,
    pub key_id: String,
    pub key_fingerprint: String,
    pub evidence_bytes: Vec<u8>,
    pub files: BTreeMap<String, Vec<u8>>,
    pub assets_known: bool,
}

/// Verify and import one protected evidence archive. Merely claiming a
/// protected source inside JSON is never sufficient: the detached signature,
/// trust root, every declared hash and release identity are checked first.
pub fn import_evidence_bundle(
    bundle_path: &Path,
    trust_registry_path: &Path,
    repository: &EvalRepository,
    artifacts: &EvalArtifactStore,
) -> Result<EvalImportResult> {
    let verified = verify_evidence_bundle(bundle_path, trust_registry_path)?;
    let bundle_artifact = artifacts.put_file(bundle_path, MAX_BUNDLE_BYTES)?;
    let evidence_artifact = artifacts.put_bytes(&verified.evidence_bytes)?;
    let mut stored_artifacts = Vec::new();
    for artifact in &verified.manifest.artifacts {
        let bytes = verified
            .files
            .get(&artifact.path)
            .ok_or_else(|| anyhow!("verified evidence artifact disappeared"))?;
        stored_artifacts.push(artifacts.put_bytes(bytes)?);
    }
    repository.import_protected_evidence(
        &bundle_artifact,
        &evidence_artifact,
        &stored_artifacts,
        &verified.key_id,
        &verified.key_fingerprint,
        &verified.evidence,
        verified.assets_known,
    )
}

/// Import a standalone model-evidence JSON for diagnostics only. The source
/// field is retained for display, but no unsigned file can acquire protected
/// integrity, become a baseline, or carry external artifact references into
/// the local store.
pub fn import_unverified_evidence_file(
    evidence_path: &Path,
    repository: &EvalRepository,
    artifacts: &EvalArtifactStore,
) -> Result<EvalImportResult> {
    let metadata = std::fs::symlink_metadata(evidence_path)
        .with_context(|| format!("reading unverified evidence {}", evidence_path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_ENTRY_BYTES
    {
        bail!("unverified evidence must be a regular JSON file no larger than 256 MiB");
    }
    let bytes = std::fs::read(evidence_path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    ha_eval_spec::model::reject_embedded_secrets(&value, "$.evidence")?;
    let evidence: ModelCampaignEvidence = serde_json::from_value(value)?;
    validate_evidence_shape(&evidence)?;
    let artifact = artifacts.put_bytes(&bytes)?;
    repository.import_unverified_evidence(&artifact, &evidence)
}

pub fn verify_evidence_bundle(
    bundle_path: &Path,
    trust_registry_path: &Path,
) -> Result<VerifiedEvidenceBundle> {
    let metadata = std::fs::symlink_metadata(bundle_path)
        .with_context(|| format!("reading evidence bundle {}", bundle_path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_BUNDLE_BYTES
    {
        bail!("evidence bundle must be a regular file no larger than 512 MiB");
    }
    let trust = load_evidence_trust_registry_file(trust_registry_path)?;

    let file = File::open(bundle_path)?;
    let mut archive = zip::ZipArchive::new(file).context("opening evidence bundle ZIP")?;
    if archive.len() > MAX_ENTRIES {
        bail!("evidence bundle contains too many entries");
    }
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name().to_string();
        if entry.is_dir()
            || entry.enclosed_name().is_none()
            || !safe_archive_path(&name)
            || entry.size() > MAX_ENTRY_BYTES
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            bail!("evidence bundle contains an unsafe entry");
        }
        let entry_size = entry.size();
        expanded = expanded
            .checked_add(entry_size)
            .ok_or_else(|| anyhow!("evidence bundle expanded size overflow"))?;
        if expanded > MAX_EXPANDED_BYTES {
            bail!("evidence bundle expands beyond 512 MiB");
        }
        let mut bytes = Vec::with_capacity(usize::try_from(entry_size)?);
        entry
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("reading evidence bundle entry")?;
        if bytes.len() as u64 != entry_size || files.insert(name, bytes).is_some() {
            bail!("evidence bundle contains a duplicate or malformed entry");
        }
    }

    let manifest_raw = files
        .get(MANIFEST_PATH)
        .ok_or_else(|| anyhow!("evidence bundle is missing manifest.json"))?;
    let manifest_value: serde_json::Value =
        serde_json::from_slice(manifest_raw).context("decoding evidence bundle manifest")?;
    let manifest: EvidenceBundleManifest = serde_json::from_value(manifest_value.clone())?;
    validate_evidence_bundle_manifest(&manifest)?;
    if manifest.repository != OFFICIAL_REPOSITORY
        || !manifest.workflow.starts_with(OFFICIAL_WORKFLOW_PREFIX)
        || !manifest
            .workflow_run_id
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        bail!("evidence bundle provenance is not an approved Hope workflow");
    }
    let signature_raw = files
        .get(SIGNATURE_PATH)
        .ok_or_else(|| anyhow!("evidence bundle is missing manifest.sig"))?;
    let signature_text = std::str::from_utf8(signature_raw)?.trim();
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature_text)
        .context("decoding evidence bundle signature")?;
    let key_fingerprint =
        verify_manifest_signature(&manifest_value, &manifest, &signature, &trust)?;

    let mut declared = BTreeSet::from([MANIFEST_PATH.to_string(), SIGNATURE_PATH.to_string()]);
    for artifact in std::iter::once(&manifest.evidence).chain(manifest.artifacts.iter()) {
        declared.insert(artifact.path.clone());
        let bytes = files
            .get(&artifact.path)
            .ok_or_else(|| anyhow!("evidence bundle is missing {}", artifact.path))?;
        if ha_eval_spec::sha256_bytes(bytes) != artifact.sha256 {
            bail!(
                "evidence bundle artifact hash mismatch for {}",
                artifact.path
            );
        }
    }
    if files.keys().any(|path| !declared.contains(path)) {
        bail!("evidence bundle contains undeclared files");
    }

    let evidence_bytes = files
        .get(&manifest.evidence.path)
        .cloned()
        .ok_or_else(|| anyhow!("evidence file is missing"))?;
    let evidence: ModelCampaignEvidence =
        serde_json::from_slice(&evidence_bytes).context("decoding protected model evidence")?;
    validate_evidence_shape(&evidence)?;
    if !evidence.source.is_release_eligible()
        || evidence.dirty
        || evidence.commit_sha != manifest.commit_sha
        || evidence.tier != manifest.tier
        || manifest.environment != "model-eval"
    {
        bail!("signed evidence does not have a protected clean exact-SHA identity");
    }
    if evidence.artifacts.len() != manifest.artifacts.len()
        || evidence.artifacts.iter().any(|artifact| {
            !manifest.artifacts.iter().any(|declared| {
                declared.path == artifact.path && declared.sha256 == artifact.sha256
            })
        })
    {
        bail!("bundle artifact manifest does not match model evidence");
    }

    let assets_known = evidence_assets_known(&evidence, trust_registry_path)?;
    Ok(VerifiedEvidenceBundle {
        key_id: manifest.key_id.clone(),
        key_fingerprint,
        manifest,
        evidence,
        evidence_bytes,
        files,
        assets_known,
    })
}

fn evidence_assets_known(
    evidence: &ModelCampaignEvidence,
    trust_registry_path: &Path,
) -> Result<bool> {
    let Some(live_root) = trust_registry_path.parent().and_then(Path::parent) else {
        return Ok(false);
    };
    let lock_path = live_root.join("version-lock.json");
    let metadata = match std::fs::symlink_metadata(&lock_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4 * 1024 * 1024
    {
        return Ok(false);
    }
    let lock: serde_json::Value = serde_json::from_slice(&std::fs::read(lock_path)?)?;
    let matches = |section: &str, key: &str, digest: &str| {
        lock.get(section)
            .and_then(serde_json::Value::as_object)
            .and_then(|values| values.get(key))
            .and_then(serde_json::Value::as_str)
            == Some(digest)
    };
    if !matches(
        "policies",
        &format!("{}@{}", evidence.policy_id, evidence.policy_version),
        &evidence.policy_digest,
    ) {
        return Ok(false);
    }
    for suite in &evidence.suites {
        if !matches(
            "suites",
            &format!("{}@{}", suite.id, suite.version),
            &suite.digest,
        ) {
            return Ok(false);
        }
        for case in &suite.cases {
            if !matches(
                "scenarios",
                &format!("{}@{}", case.scenario_id, case.scenario_version),
                &case.scenario_digest,
            ) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn verify_manifest_signature(
    manifest_value: &serde_json::Value,
    manifest: &EvidenceBundleManifest,
    signature: &[u8],
    trust: &EvidenceTrustRegistry,
) -> Result<String> {
    let key = trust
        .keys
        .iter()
        .find(|candidate| candidate.id == manifest.key_id)
        .ok_or_else(|| anyhow!("evidence signing key is not trusted"))?;
    validate_key_for_import(
        key.status,
        &key.valid_from,
        key.valid_until.as_deref(),
        &manifest.created_at,
    )?;
    let public_key = base64::engine::general_purpose::STANDARD
        .decode(&key.public_key)
        .context("decoding evidence public key")?;
    if public_key.len() != 32 || signature.len() != 64 {
        bail!("evidence Ed25519 key or signature has an invalid size");
    }
    let canonical = canonical_json(manifest_value)?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(&canonical, signature)
        .map_err(|_| anyhow!("evidence bundle signature verification failed"))?;
    evidence_trust_key_fingerprint(key)
}

fn validate_key_for_import(
    status: EvidenceKeyStatus,
    valid_from: &str,
    valid_until: Option<&str>,
    signed_at: &str,
) -> Result<()> {
    if status == EvidenceKeyStatus::Revoked {
        bail!("evidence signing key is revoked");
    }
    let from = DateTime::parse_from_rfc3339(valid_from)?;
    let signed = DateTime::parse_from_rfc3339(signed_at)?;
    if signed < from || signed > Utc::now() + chrono::Duration::minutes(5) {
        bail!("evidence signature timestamp is outside the key validity window");
    }
    if let Some(until) = valid_until {
        if signed > DateTime::parse_from_rfc3339(until)? {
            bail!("evidence was signed after the key validity window");
        }
    } else if status == EvidenceKeyStatus::Retired {
        bail!("retired evidence keys must record validUntil");
    }
    Ok(())
}

fn safe_archive_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !value.contains('\\')
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_eval_spec::app::{
        EvidenceTrustKey, EVIDENCE_BUNDLE_SCHEMA_VERSION, EVIDENCE_TRUST_SCHEMA_VERSION,
    };
    use ha_eval_spec::model::ModelCampaignTier;
    use ha_eval_spec::ArtifactDigest;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    #[test]
    fn detached_signature_fails_closed_after_manifest_tampering() {
        let key_pair = Ed25519KeyPair::from_pkcs8(
            Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let now = Utc::now().to_rfc3339();
        let manifest = EvidenceBundleManifest {
            schema_version: EVIDENCE_BUNDLE_SCHEMA_VERSION.to_string(),
            repository: "hope-agent".to_string(),
            workflow: "model-campaign".to_string(),
            workflow_run_id: "123".to_string(),
            environment: "model-eval".to_string(),
            commit_sha: "a".repeat(40),
            tier: ModelCampaignTier::Weekly,
            created_at: now.clone(),
            key_id: "test-key".to_string(),
            evidence: ArtifactDigest {
                path: "evidence.json".to_string(),
                sha256: "b".repeat(64),
            },
            artifacts: Vec::new(),
        };
        let trust = EvidenceTrustRegistry {
            schema_version: EVIDENCE_TRUST_SCHEMA_VERSION.to_string(),
            version: "1.0.0".to_string(),
            keys: vec![EvidenceTrustKey {
                id: "test-key".to_string(),
                algorithm: "ed25519".to_string(),
                public_key: base64::engine::general_purpose::STANDARD
                    .encode(key_pair.public_key().as_ref()),
                status: EvidenceKeyStatus::Active,
                valid_from: (Utc::now() - chrono::Duration::minutes(1)).to_rfc3339(),
                valid_until: None,
                revoked_at: None,
            }],
        };
        let value = serde_json::to_value(&manifest).unwrap();
        let signature = key_pair.sign(&canonical_json(&value).unwrap());
        verify_manifest_signature(&value, &manifest, signature.as_ref(), &trust).unwrap();

        let mut tampered = value.clone();
        tampered["commitSha"] = serde_json::Value::String("c".repeat(40));
        let tampered_manifest: EvidenceBundleManifest =
            serde_json::from_value(tampered.clone()).unwrap();
        assert!(verify_manifest_signature(
            &tampered,
            &tampered_manifest,
            signature.as_ref(),
            &trust
        )
        .is_err());

        let mut revoked = trust;
        revoked.keys[0].status = EvidenceKeyStatus::Revoked;
        revoked.keys[0].revoked_at = Some(now);
        assert!(
            verify_manifest_signature(&value, &manifest, signature.as_ref(), &revoked).is_err()
        );
    }

    #[test]
    fn archive_paths_reject_traversal_and_platform_separators() {
        assert!(safe_archive_path("artifacts/trace.jsonl"));
        assert!(!safe_archive_path("../trace.jsonl"));
        assert!(!safe_archive_path("artifacts\\trace.jsonl"));
        assert!(!safe_archive_path("/absolute/trace.jsonl"));
    }
}
