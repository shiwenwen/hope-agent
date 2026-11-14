//! Owner-only, bounded review captures and baseline revision history.
//! A single atomic manifest publishes references; PNGs are installed first and
//! garbage collected only after publication. Callers hold the lifecycle lock.
use super::quality::{
    self, AcceptBaselineInput, BaselineEntry, QualityManifest, QualityRun, QualityViewport,
};
use anyhow::{Context, Result};
use base64::Engine;
use ha_core::platform::write_atomic;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};

const MAX_REVIEWS: usize = 4;
const MAX_REVISIONS: usize = 4;
const MAX_STORE_BYTES: u64 = 768 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityReceipt {
    pub created_at: String,
    pub run: QualityRun,
    pub entries: BTreeMap<String, BaselineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineRevision {
    pub id: String,
    pub parent_id: Option<String>,
    pub restored_from: Option<String>,
    pub created_at: String,
    pub baselines: BTreeMap<String, BaselineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionSummary {
    pub id: String,
    pub parent_id: Option<String>,
    pub restored_from: Option<String>,
    pub created_at: String,
}

pub(super) fn revision_summaries(manifest: &QualityManifest) -> Vec<RevisionSummary> {
    manifest
        .revisions
        .iter()
        .map(|r| RevisionSummary {
            id: r.id.clone(),
            parent_id: r.parent_id.clone(),
            restored_from: r.restored_from.clone(),
            created_at: r.created_at.clone(),
        })
        .collect()
}

pub(super) fn locked_artifact(id: &str) -> Result<(super::db::DesignArtifact, std::fs::File)> {
    let candidate = super::service::get_artifact(id)?.context("artifact not found")?;
    let guard =
        super::service::acquire_artifact_lifecycle_lock(&candidate.project_id, &candidate.id)?;
    let current = super::service::get_artifact(id)?.context("artifact not found")?;
    anyhow::ensure!(
        current.project_id == candidate.project_id,
        "artifact identity changed"
    );
    Ok((current, guard))
}

pub(super) fn read_bounded(path: &Path, max: usize) -> std::io::Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid quality file or size",
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "quality file exceeds limit",
        ));
    }
    Ok(bytes)
}

pub(super) fn storage_dirs(dir: &Path) -> Result<()> {
    for path in [dir.to_path_buf(), dir.join("screenshots")] {
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => anyhow::ensure!(
                meta.is_dir() && !meta.file_type().is_symlink(),
                "invalid quality directory"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn screenshot(dir: &Path, entry: &BaselineEntry) -> Result<Vec<u8>> {
    storage_dirs(dir)?;
    anyhow::ensure!(valid_hash(&entry.image_hash), "invalid screenshot identity");
    let bytes = read_bounded(
        &dir.join("screenshots")
            .join(format!("{}.png", entry.image_hash)),
        quality::MAX_PNG_BYTES,
    )?;
    anyhow::ensure!(
        blake3::hash(&bytes).to_hex().as_str() == entry.image_hash,
        "screenshot content hash mismatch"
    );
    quality::capture_image(&bytes, entry.viewport)?;
    Ok(bytes)
}

fn references(manifest: &QualityManifest) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    refs.extend(manifest.baselines.values().map(|e| e.image_hash.clone()));
    for revision in &manifest.revisions {
        refs.extend(revision.baselines.values().map(|e| e.image_hash.clone()));
    }
    for review in &manifest.reviews {
        refs.extend(review.entries.values().map(|e| e.image_hash.clone()));
        refs.extend(
            review
                .run
                .diffs
                .iter()
                .filter_map(|d| d.baseline_hash.clone()),
        );
    }
    refs
}

fn collect_unreferenced(dir: &Path, refs: &BTreeSet<String>) -> Result<()> {
    let shots = dir.join("screenshots");
    if !shots.exists() {
        return Ok(());
    }
    for file in std::fs::read_dir(shots)? {
        let file = file?;
        let name = file.file_name();
        let Some(hash) = name.to_str().and_then(|n| n.strip_suffix(".png")) else {
            continue;
        };
        if valid_hash(hash) && !refs.contains(hash) && file.file_type()?.is_file() {
            std::fs::remove_file(file.path())?;
        }
    }
    Ok(())
}

pub(super) fn publish(
    dir: &Path,
    manifest: &mut QualityManifest,
    images: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    storage_dirs(dir)?;
    // Remove only images unreferenced by the last durable manifest before staging.
    let prior = quality::load_manifest(dir, &manifest.artifact_id)?;
    collect_unreferenced(dir, &references(&prior))?;
    if manifest.reviews.len() > MAX_REVIEWS {
        manifest
            .reviews
            .drain(..manifest.reviews.len() - MAX_REVIEWS);
    }
    if manifest.revisions.len() > MAX_REVISIONS {
        manifest
            .revisions
            .drain(..manifest.revisions.len() - MAX_REVISIONS);
    }
    let refs = references(manifest);
    let mut total = 0_u64;
    for hash in &refs {
        anyhow::ensure!(valid_hash(hash), "invalid screenshot identity");
        let len = if let Some(bytes) = images.get(hash) {
            anyhow::ensure!(
                blake3::hash(bytes).to_hex().as_str() == hash,
                "staged screenshot hash mismatch"
            );
            bytes.len() as u64
        } else {
            let meta =
                std::fs::symlink_metadata(dir.join("screenshots").join(format!("{hash}.png")))?;
            anyhow::ensure!(
                meta.is_file()
                    && !meta.file_type().is_symlink()
                    && meta.len() <= quality::MAX_PNG_BYTES as u64,
                "invalid referenced screenshot"
            );
            meta.len()
        };
        total = total.checked_add(len).context("quality storage overflow")?;
    }
    anyhow::ensure!(
        total <= MAX_STORE_BYTES,
        "quality storage budget exceeded; retained baseline images are protected"
    );
    manifest.version = 2;
    let bytes = serde_json::to_vec_pretty(manifest)?;
    anyhow::ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "quality manifest exceeds limit"
    );
    std::fs::create_dir_all(dir.join("screenshots"))?;
    for (hash, png) in images {
        anyhow::ensure!(
            valid_hash(hash) && refs.contains(hash) && png.len() <= quality::MAX_PNG_BYTES,
            "invalid staged screenshot"
        );
        write_atomic(&dir.join("screenshots").join(format!("{hash}.png")), png)?;
    }
    write_atomic(&dir.join("manifest.json"), &bytes)?;
    // Publication already succeeded. Cleanup failure must not report a failed
    // acceptance after changing the baseline; retry cleanup on the next write.
    if let Err(error) = collect_unreferenced(dir, &refs) {
        ha_core::app_warn!(
            "design",
            "quality_cleanup",
            "quality screenshot cleanup deferred: {error}"
        );
    }
    Ok(())
}

fn revision(
    manifest: &mut QualityManifest,
    baselines: BTreeMap<String, BaselineEntry>,
    restored_from: Option<String>,
) {
    if manifest.revision_id.is_none() && !manifest.baselines.is_empty() {
        let id = uuid::Uuid::new_v4().to_string();
        manifest.revisions.push(BaselineRevision {
            id: id.clone(),
            parent_id: None,
            restored_from: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            baselines: manifest.baselines.clone(),
        });
        manifest.revision_id = Some(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    manifest.revisions.push(BaselineRevision {
        id: id.clone(),
        parent_id: manifest.revision_id.clone(),
        restored_from,
        created_at: chrono::Utc::now().to_rfc3339(),
        baselines: baselines.clone(),
    });
    manifest.baselines = baselines;
    manifest.revision_id = Some(id);
}

pub(super) fn accept_review_at(
    dir: &Path,
    input: &AcceptBaselineInput,
    actual_hash: &str,
    scenarios_hash: &str,
    plan_keys: &BTreeSet<String>,
) -> Result<QualityManifest> {
    anyhow::ensure!(
        input.expected_artifact_hash == actual_hash,
        "stale baseline acceptance: artifact changed"
    );
    let mut manifest = quality::load_manifest(dir, &input.artifact_id)?;
    anyhow::ensure!(
        input.expected_baseline_revision == manifest.revision_id,
        "stale baseline revision"
    );
    let receipt = manifest
        .reviews
        .iter()
        .find(|r| r.run.review_id == input.review_id)
        .context("review receipt missing or expired; run review again")?;
    anyhow::ensure!(
        receipt.run.artifact_id == input.artifact_id
            && receipt.run.artifact_hash == actual_hash
            && receipt.run.scenarios_hash == scenarios_hash,
        "stale reviewed capture"
    );
    anyhow::ensure!(
        receipt.run.baseline_revision == manifest.revision_id,
        "reviewed baseline revision changed"
    );
    let keys: BTreeSet<_> = receipt.entries.keys().cloned().collect();
    anyhow::ensure!(
        !keys.is_empty() && &keys == plan_keys,
        "reviewed capture set mismatch"
    );
    let mut baselines = receipt.entries.clone();
    for entry in baselines.values_mut() {
        anyhow::ensure!(
            entry.artifact_hash == actual_hash,
            "reviewed capture artifact mismatch"
        );
        screenshot(dir, entry)?;
        entry.accepted_at = chrono::Utc::now().to_rfc3339();
    }
    revision(&mut manifest, baselines, None);
    publish(dir, &mut manifest, &BTreeMap::new())?;
    Ok(manifest)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityImageInput {
    pub artifact_id: String,
    pub review_id: String,
    pub scenario_id: String,
    pub viewport: QualityViewport,
    pub side: ImageSide,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageSide {
    Current,
    Baseline,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityImage {
    pub data_url: String,
    pub image_hash: String,
}

fn review_image_at(dir: &Path, input: &QualityImageInput) -> Result<QualityImage> {
    let manifest = quality::load_manifest(dir, &input.artifact_id)?;
    let review = manifest
        .reviews
        .iter()
        .find(|r| r.run.review_id == input.review_id)
        .context("review receipt missing or expired")?;
    anyhow::ensure!(
        review.run.artifact_id == input.artifact_id,
        "review artifact identity mismatch"
    );
    let diff = review
        .run
        .diffs
        .iter()
        .find(|d| d.scenario_id == input.scenario_id && d.viewport == input.viewport)
        .context("review capture missing")?;
    let image_hash = match input.side {
        ImageSide::Current => diff.current_hash.clone(),
        ImageSide::Baseline => diff
            .baseline_hash
            .clone()
            .context("review has no baseline")?,
    };
    let entry = BaselineEntry {
        scenario_id: input.scenario_id.clone(),
        viewport: input.viewport,
        image_hash: image_hash.clone(),
        artifact_hash: review.run.artifact_hash.clone(),
        accepted_at: review.created_at.clone(),
    };
    let bytes = screenshot(dir, &entry)?;
    Ok(QualityImage {
        data_url: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ),
        image_hash,
    })
}

pub fn review_image(input: QualityImageInput) -> Result<QualityImage> {
    let (artifact, _guard) = locked_artifact(&input.artifact_id)?;
    let dir =
        ha_core::paths::design_artifact_dir(&artifact.project_id, &artifact.id)?.join("quality");
    review_image_at(&dir, &input)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreBaselineInput {
    pub artifact_id: String,
    pub revision_id: String,
    pub expected_revision_id: String,
}
fn restore_at(dir: &Path, input: &RestoreBaselineInput) -> Result<QualityManifest> {
    let mut manifest = quality::load_manifest(dir, &input.artifact_id)?;
    anyhow::ensure!(
        manifest.revision_id.as_deref() == Some(&input.expected_revision_id),
        "stale baseline revision"
    );
    anyhow::ensure!(
        input.revision_id != input.expected_revision_id,
        "baseline revision already current"
    );
    let target = manifest
        .revisions
        .iter()
        .find(|r| r.id == input.revision_id)
        .context("baseline revision missing or expired")?;
    for entry in target.baselines.values() {
        screenshot(dir, entry)?;
    }
    let baselines = target.baselines.clone();
    revision(&mut manifest, baselines, Some(input.revision_id.clone()));
    publish(dir, &mut manifest, &BTreeMap::new())?;
    Ok(manifest)
}
pub fn restore(input: RestoreBaselineInput) -> Result<QualityManifest> {
    let (artifact, _guard) = locked_artifact(&input.artifact_id)?;
    let dir =
        ha_core::paths::design_artifact_dir(&artifact.project_id, &artifact.id)?.join("quality");
    restore_at(&dir, &input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quality::{StaticFinding, ViewportDiff};

    fn save_review(dir: &Path, color: u8) -> (QualityRun, String) {
        let mut manifest = quality::load_manifest(dir, "A").unwrap();
        let viewport = QualityViewport {
            width: 2,
            height: 2,
        };
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([color, 0, 0, 255]),
        ));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = png.into_inner();
        let hash = blake3::hash(&png).to_hex().to_string();
        let run = QualityRun {
            review_id: uuid::Uuid::new_v4().to_string(),
            baseline_revision: manifest.revision_id.clone(),
            baseline_revisions: revision_summaries(&manifest),
            artifact_id: "A".into(),
            artifact_hash: "source".into(),
            scenarios_hash: "scenes".into(),
            diffs: vec![ViewportDiff {
                scenario_id: "default".into(),
                scenario_title: "Default".into(),
                viewport,
                current_hash: hash.clone(),
                baseline_hash: manifest.baselines.get("2x2").map(|b| b.image_hash.clone()),
                changed_ratio: None,
                mean_delta: None,
                passed: false,
            }],
            findings: Vec::<StaticFinding>::new(),
            deterministic_passed: false,
        };
        manifest.reviews.push(QualityReceipt {
            created_at: chrono::Utc::now().to_rfc3339(),
            run: run.clone(),
            entries: BTreeMap::from([(
                "2x2".into(),
                BaselineEntry {
                    scenario_id: "default".into(),
                    viewport,
                    image_hash: hash.clone(),
                    artifact_hash: "source".into(),
                    accepted_at: "capture".into(),
                },
            )]),
        });
        publish(dir, &mut manifest, &BTreeMap::from([(hash.clone(), png)])).unwrap();
        (run, hash)
    }
    fn input(run: &QualityRun) -> AcceptBaselineInput {
        AcceptBaselineInput {
            artifact_id: "A".into(),
            review_id: run.review_id.clone(),
            expected_artifact_hash: run.artifact_hash.clone(),
            expected_scenarios_hash: Some(run.scenarios_hash.clone()),
            expected_baseline_revision: run.baseline_revision.clone(),
        }
    }
    fn accept_at(dir: &Path, run: &QualityRun) -> QualityManifest {
        accept_review_at(
            dir,
            &input(run),
            "source",
            "scenes",
            &BTreeSet::from(["2x2".into()]),
        )
        .unwrap()
    }
    fn image_input(run: &QualityRun, side: ImageSide) -> QualityImageInput {
        QualityImageInput {
            artifact_id: "A".into(),
            review_id: run.review_id.clone(),
            scenario_id: "default".into(),
            viewport: run.diffs[0].viewport,
            side,
        }
    }

    #[test]
    fn accepts_the_exact_reviewed_bytes_and_restores_a_revision_without_capture() {
        let dir = tempfile::tempdir().unwrap();
        let (run1, hash1) = save_review(dir.path(), 10);
        let shown = review_image_at(dir.path(), &image_input(&run1, ImageSide::Current)).unwrap();
        let first = accept_at(dir.path(), &run1);
        assert_eq!(first.baselines["2x2"].image_hash, shown.image_hash);
        assert!(shown.data_url.ends_with(
            &base64::engine::general_purpose::STANDARD
                .encode(screenshot(dir.path(), &first.baselines["2x2"]).unwrap())
        ));
        let (run2, hash2) = save_review(dir.path(), 200);
        assert_eq!(
            review_image_at(dir.path(), &image_input(&run2, ImageSide::Baseline))
                .unwrap()
                .image_hash,
            hash1
        );
        let second = accept_at(dir.path(), &run2);
        assert_eq!(second.baselines["2x2"].image_hash, hash2);
        let request = RestoreBaselineInput {
            artifact_id: "A".into(),
            revision_id: first.revision_id.unwrap(),
            expected_revision_id: second.revision_id.unwrap(),
        };
        let restored = restore_at(dir.path(), &request).unwrap();
        assert_eq!(restored.baselines["2x2"].image_hash, hash1);
        let revision = restored.revisions.last().unwrap();
        assert_eq!(
            revision.parent_id.as_ref(),
            Some(&request.expected_revision_id)
        );
        assert_eq!(revision.restored_from.as_ref(), Some(&request.revision_id));
        assert!(restore_at(dir.path(), &request).is_err());
    }

    #[test]
    fn stale_source_scenarios_revision_and_incomplete_set_leave_baseline_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let (run, _) = save_review(dir.path(), 12);
        let before = std::fs::read(dir.path().join("manifest.json")).unwrap();
        let keys = BTreeSet::from(["2x2".into()]);
        assert!(accept_review_at(dir.path(), &input(&run), "changed", "scenes", &keys).is_err());
        assert!(accept_review_at(dir.path(), &input(&run), "source", "changed", &keys).is_err());
        assert!(accept_review_at(
            dir.path(),
            &input(&run),
            "source",
            "scenes",
            &BTreeSet::new()
        )
        .is_err());
        let mut wrong = input(&run);
        wrong.artifact_id = "B".into();
        assert!(accept_review_at(dir.path(), &wrong, "source", "scenes", &keys).is_err());
        assert_eq!(
            before,
            std::fs::read(dir.path().join("manifest.json")).unwrap()
        );
        accept_at(dir.path(), &run);
        assert!(accept_review_at(dir.path(), &input(&run), "source", "scenes", &keys).is_err());
    }

    #[test]
    fn image_access_is_receipt_scoped_and_rejects_unknown_identity_or_corrupt_png() {
        let dir = tempfile::tempdir().unwrap();
        let (run, hash) = save_review(dir.path(), 1);
        let mut request = image_input(&run, ImageSide::Current);
        request.artifact_id = "B".into();
        assert!(review_image_at(dir.path(), &request).is_err());
        request.artifact_id = "A".into();
        request.scenario_id = "other".into();
        assert!(review_image_at(dir.path(), &request).is_err());
        request.scenario_id = "default".into();
        request.review_id = "../../manifest".into();
        assert!(review_image_at(dir.path(), &request).is_err());
        assert!(review_image_at(dir.path(), &image_input(&run, ImageSide::Baseline)).is_err());
        let before = std::fs::read(dir.path().join("manifest.json")).unwrap();
        std::fs::write(
            dir.path().join("screenshots").join(format!("{hash}.png")),
            b"corrupt",
        )
        .unwrap();
        assert!(review_image_at(dir.path(), &image_input(&run, ImageSide::Current)).is_err());
        assert!(accept_review_at(
            dir.path(),
            &input(&run),
            "source",
            "scenes",
            &BTreeSet::from(["2x2".into()])
        )
        .is_err());
        assert_eq!(
            before,
            std::fs::read(dir.path().join("manifest.json")).unwrap()
        );
    }

    #[test]
    fn retention_keeps_referenced_images_and_expires_old_receipts_and_revisions() {
        let dir = tempfile::tempdir().unwrap();
        let mut runs = Vec::new();
        for color in 0..7 {
            let (run, _) = save_review(dir.path(), color);
            accept_at(dir.path(), &run);
            runs.push(run);
        }
        let manifest = quality::load_manifest(dir.path(), "A").unwrap();
        assert_eq!(manifest.reviews.len(), MAX_REVIEWS);
        assert_eq!(manifest.revisions.len(), MAX_REVISIONS);
        assert!(review_image_at(dir.path(), &image_input(&runs[0], ImageSide::Current)).is_err());
        for hash in references(&manifest) {
            assert!(dir
                .path()
                .join("screenshots")
                .join(format!("{hash}.png"))
                .is_file());
        }
        let orphan = dir
            .path()
            .join("screenshots")
            .join(format!("{}.png", "f".repeat(64)));
        std::fs::write(&orphan, b"orphan").unwrap();
        let mut manifest = manifest;
        publish(dir.path(), &mut manifest, &BTreeMap::new()).unwrap();
        assert!(!orphan.exists());
        assert!(restore_at(
            dir.path(),
            &RestoreBaselineInput {
                artifact_id: "A".into(),
                revision_id: "expired".into(),
                expected_revision_id: manifest.revision_id.unwrap()
            }
        )
        .is_err());
    }

    #[test]
    fn accepting_a_legacy_baseline_keeps_it_as_a_restorable_revision() {
        let dir = tempfile::tempdir().unwrap();
        let (run, _) = save_review(dir.path(), 7);
        let mut manifest = quality::load_manifest(dir.path(), "A").unwrap();
        manifest.version = 1;
        manifest.baselines = manifest.reviews[0].entries.clone();
        std::fs::write(
            dir.path().join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let accepted = accept_at(dir.path(), &run);
        assert_eq!(accepted.version, 2);
        assert_eq!(accepted.revisions.len(), 2);
        assert_eq!(
            accepted.revisions[1].parent_id.as_ref(),
            Some(&accepted.revisions[0].id)
        );
        assert_eq!(
            serde_json::to_value(&accepted.revisions[0].baselines).unwrap(),
            serde_json::to_value(&manifest.baselines).unwrap()
        );
    }

    #[test]
    fn storage_budget_rejection_preserves_the_durable_manifest_and_referenced_images() {
        let dir = tempfile::tempdir().unwrap();
        let (run, _) = save_review(dir.path(), 1);
        let mut manifest = quality::load_manifest(dir.path(), "A").unwrap();
        for n in 0..25 {
            let hash = format!("{n:064x}");
            let file =
                std::fs::File::create(dir.path().join("screenshots").join(format!("{hash}.png")))
                    .unwrap();
            file.set_len(quality::MAX_PNG_BYTES as u64).unwrap();
            let mut entry = manifest.reviews[0].entries["2x2"].clone();
            entry.image_hash = hash;
            manifest.baselines.insert(format!("old-{n}"), entry);
        }
        // An oversized legacy store is retained, never silently trimmed.
        let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        std::fs::write(dir.path().join("manifest.json"), &bytes).unwrap();
        assert!(publish(dir.path(), &mut manifest, &BTreeMap::new())
            .unwrap_err()
            .to_string()
            .contains("budget exceeded"));
        assert_eq!(
            std::fs::read(dir.path().join("manifest.json")).unwrap(),
            bytes
        );
        assert!(review_image_at(dir.path(), &image_input(&run, ImageSide::Current)).is_ok());
    }

    #[test]
    fn legacy_manifest_and_invalid_hashes_or_dimensions_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("manifest.json"),
            br#"{"version":1,"artifactId":"A","baselines":{}}"#,
        )
        .unwrap();
        assert!(quality::load_manifest(dir.path(), "A")
            .unwrap()
            .revisions
            .is_empty());
        let (run, _) = save_review(dir.path(), 2);
        let mut entry =
            quality::load_manifest(dir.path(), "A").unwrap().reviews[0].entries["2x2"].clone();
        entry.image_hash = "../manifest".into();
        assert!(screenshot(dir.path(), &entry).is_err());
        entry.image_hash = run.diffs[0].current_hash.clone();
        entry.viewport.width = 3;
        assert!(screenshot(dir.path(), &entry).is_err());
        #[cfg(unix)]
        {
            let link = dir.path().join("linked");
            std::os::unix::fs::symlink(dir.path().join("screenshots"), &link).unwrap();
            assert!(read_bounded(&link, 100).is_err());
        }
    }
}
