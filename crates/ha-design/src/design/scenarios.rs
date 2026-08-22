//! 产物预览场景清单：有界输入、固定视口与可验证的本地 JSON 真相源。

use anyhow::{Context, Result};
use ha_core::platform::write_atomic;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MAX_SCENARIOS: usize = 12;
const MAX_VIEWPORTS: usize = 4;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_STATE_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioViewport {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignScenario {
    pub id: String,
    pub title: String,
    #[serde(default = "default_route")]
    pub route: String,
    #[serde(default)]
    pub state: Value,
    #[serde(default)]
    pub viewport_ids: Vec<String>,
}

fn default_route() -> String {
    "/".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenariosManifest {
    pub version: u32,
    #[serde(default)]
    pub viewports: std::collections::BTreeMap<String, ScenarioViewport>,
    #[serde(default)]
    pub scenarios: Vec<DesignScenario>,
}

impl Default for ScenariosManifest {
    fn default() -> Self {
        Self {
            version: 1,
            viewports: std::collections::BTreeMap::new(),
            scenarios: vec![DesignScenario {
                id: "default".into(),
                title: "默认".into(),
                route: "/".into(),
                state: Value::Object(Default::default()),
                viewport_ids: Vec::new(),
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenariosEnvelope {
    pub manifest: ScenariosManifest,
    pub hash: String,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn validate(manifest: &ScenariosManifest) -> Result<()> {
    if manifest.version != 1 {
        anyhow::bail!("unsupported scenarios manifest version");
    }
    if manifest.scenarios.is_empty() || manifest.scenarios.len() > MAX_SCENARIOS {
        anyhow::bail!("scenarios must contain 1..={MAX_SCENARIOS} entries");
    }
    if manifest.viewports.len() > MAX_VIEWPORTS {
        anyhow::bail!("at most {MAX_VIEWPORTS} viewports are allowed");
    }
    for (id, viewport) in &manifest.viewports {
        if !valid_id(id)
            || !(240..=2560).contains(&viewport.width)
            || !(240..=2560).contains(&viewport.height)
        {
            anyhow::bail!("invalid scenario viewport");
        }
    }
    let mut ids = std::collections::BTreeSet::new();
    for scenario in &manifest.scenarios {
        if !valid_id(&scenario.id) || !ids.insert(&scenario.id) || scenario.title.len() > 120 {
            anyhow::bail!("invalid or duplicate scenario id/title");
        }
        if !scenario.route.starts_with('/')
            || scenario.route.contains("://")
            || scenario.route.len() > 512
        {
            anyhow::bail!("scenario route must be a local path");
        }
        if serde_json::to_vec(&scenario.state)?.len() > MAX_STATE_BYTES {
            anyhow::bail!("scenario state exceeds 8 KiB");
        }
        if !scenario.state.is_object() && !scenario.state.is_null() {
            anyhow::bail!("scenario state must be a JSON object");
        }
        if scenario
            .viewport_ids
            .iter()
            .any(|id| !manifest.viewports.contains_key(id))
        {
            anyhow::bail!("scenario references an unknown viewport");
        }
    }
    let bytes = serde_json::to_vec(manifest)?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!("scenarios manifest exceeds 64 KiB");
    }
    Ok(())
}

fn path(artifact_id: &str) -> Result<PathBuf> {
    let artifact = super::service::get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    Ok(
        ha_core::paths::design_artifact_dir(&artifact.project_id, artifact_id)?
            .join("scenarios.json"),
    )
}

fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn read_one(path: &Path) -> Result<ScenariosEnvelope> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            serde_json::to_vec_pretty(&ScenariosManifest::default())?
        }
        Err(e) => return Err(e).context("read scenarios manifest"),
    };
    let manifest: ScenariosManifest = serde_json::from_slice(&bytes)?;
    validate(&manifest)?;
    Ok(ScenariosEnvelope {
        manifest,
        hash: hash_bytes(&bytes),
    })
}

fn manifest_lock_path(path: &Path) -> PathBuf {
    path.with_extension("update.lock")
}

fn acquire_manifest_lock(path: &Path) -> Result<std::fs::File> {
    ha_core::platform::try_acquire_exclusive_lock(&manifest_lock_path(path))?
        .ok_or_else(|| anyhow::anyhow!("scenarios manifest update already in progress"))
}

fn save_to_path(
    path: &Path,
    expected_hash: &str,
    manifest: ScenariosManifest,
) -> Result<ScenariosEnvelope> {
    validate(&manifest)?;
    let _manifest_guard = acquire_manifest_lock(path)?;
    let current = read_one(path)?;
    if current.hash != expected_hash {
        anyhow::bail!("stale scenarios manifest: saved version changed");
    }
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    write_atomic(path, &bytes)?;
    Ok(ScenariosEnvelope {
        manifest,
        hash: hash_bytes(&bytes),
    })
}

fn save_to_path_after_lifecycle_lock<A, R>(
    path: &Path,
    expected_hash: &str,
    manifest: ScenariosManifest,
    acquire_lifecycle_lock: A,
    revalidate_artifact: R,
) -> Result<ScenariosEnvelope>
where
    A: FnOnce() -> Result<std::fs::File>,
    R: FnOnce() -> Result<()>,
{
    validate(&manifest)?;
    let _lifecycle_guard = acquire_lifecycle_lock()?;
    // The artifact may have been deleted while this writer waited for a
    // different process. Revalidate only after owning the same stable lock as
    // deletion, before write_atomic can recreate any missing parent directory.
    revalidate_artifact()?;
    save_to_path(path, expected_hash, manifest)
}

pub fn get(artifact_id: &str) -> Result<ScenariosEnvelope> {
    read_one(&path(artifact_id)?)
}

pub fn save(
    artifact_id: &str,
    expected_hash: &str,
    manifest: ScenariosManifest,
) -> Result<ScenariosEnvelope> {
    let process_lock = super::service::artifact_lock(artifact_id);
    let _process_guard = process_lock.lock().unwrap_or_else(|e| e.into_inner());
    let artifact = super::service::get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let project_id = artifact.project_id;
    let scenarios_path =
        ha_core::paths::design_artifact_dir(&project_id, artifact_id)?.join("scenarios.json");

    save_to_path_after_lifecycle_lock(
        &scenarios_path,
        expected_hash,
        manifest,
        || super::service::acquire_artifact_lifecycle_lock(&project_id, artifact_id),
        || {
            let current = super::service::get_artifact(artifact_id)?
                .with_context(|| format!("artifact not found: {artifact_id}"))?;
            if current.project_id != project_id {
                anyhow::bail!("artifact identity changed while waiting for lifecycle lock");
            }
            Ok(())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_remote_routes_and_unbounded_counts() {
        let mut manifest = ScenariosManifest::default();
        manifest.scenarios[0].route = "https://example.com".into();
        assert!(validate(&manifest).is_err());
        manifest.scenarios = (0..13)
            .map(|i| DesignScenario {
                id: format!("s{i}"),
                title: format!("S{i}"),
                route: "/".into(),
                state: serde_json::json!({}),
                viewport_ids: vec![],
            })
            .collect();
        assert!(validate(&manifest).is_err());
    }

    #[test]
    fn stale_whole_manifest_save_is_rejected_without_losing_the_winner() {
        let directory = tempfile::tempdir().expect("scenarios tempdir");
        let path = directory.path().join("scenarios.json");
        let original = read_one(&path).expect("default manifest");

        let mut first = original.manifest.clone();
        first.scenarios[0].title = "first writer".to_string();
        let first = save_to_path(&path, &original.hash, first).expect("first save");

        let mut stale = original.manifest;
        stale.scenarios[0].title = "stale writer".to_string();
        let error =
            save_to_path(&path, &original.hash, stale).expect_err("stale save must fail closed");
        assert!(error.to_string().contains("stale scenarios manifest"));

        let current = read_one(&path).expect("current manifest");
        assert_eq!(current.hash, first.hash);
        assert_eq!(current.manifest.scenarios[0].title, "first writer");
    }

    #[test]
    fn deletion_wins_the_lifecycle_lock_without_recreating_an_orphan() {
        let directory = tempfile::tempdir().expect("scenarios lifecycle tempdir");
        let artifact_dir = directory.path().join("artifact");
        std::fs::create_dir_all(&artifact_dir).expect("create artifact directory");
        let scenarios_path = artifact_dir.join("scenarios.json");
        let original = read_one(&scenarios_path).expect("default manifest");
        let lifecycle_lock_path = directory.path().join("artifact-lifecycle.lock");
        let deletion_guard =
            super::super::service::acquire_artifact_lifecycle_lock_at(lifecycle_lock_path.clone())
                .expect("deletion lifecycle lock");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let saved_artifact_dir = artifact_dir.clone();
        let saved_scenarios_path = scenarios_path.clone();
        let saver = std::thread::spawn(move || {
            started_tx.send(()).expect("signal saver start");
            save_to_path_after_lifecycle_lock(
                &saved_scenarios_path,
                &original.hash,
                original.manifest,
                || super::super::service::acquire_artifact_lifecycle_lock_at(lifecycle_lock_path),
                || {
                    anyhow::ensure!(
                        saved_artifact_dir.exists(),
                        "artifact not found after delete"
                    );
                    Ok(())
                },
            )
        });

        started_rx.recv().expect("saver started");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !saver.is_finished(),
            "scenario save must wait behind artifact deletion"
        );
        std::fs::remove_dir_all(&artifact_dir).expect("delete artifact directory");
        drop(deletion_guard);

        let error = saver
            .join()
            .expect("scenario saver thread")
            .expect_err("deleted artifact must reject the waiting save");
        assert!(error
            .to_string()
            .contains("artifact not found after delete"));
        assert!(
            !artifact_dir.exists(),
            "rejected save must not recreate an orphaned artifact directory"
        );
    }
}
