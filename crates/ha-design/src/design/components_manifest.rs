//! 组件清单：本地代码只读扫描、草稿与 expected-hash 发布。

use anyhow::{Context, Result};
use ha_core::platform::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_COMPONENTS: usize = 1_000;
const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentMode {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub props: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentEntry {
    pub id: String,
    pub name: String,
    pub import_path: String,
    #[serde(default)]
    pub export_name: Option<String>,
    #[serde(default)]
    pub modes: Vec<ComponentMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentsManifest {
    pub version: u32,
    #[serde(default)]
    pub components: Vec<ComponentEntry>,
}

impl Default for ComponentsManifest {
    fn default() -> Self {
        Self {
            version: 1,
            components: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEnvelope {
    pub manifest: ComponentsManifest,
    pub hash: String,
    pub draft: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishManifestInput {
    pub project_id: String,
    pub expected_published_hash: String,
    pub expected_draft_hash: String,
    pub manifest: ComponentsManifest,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn validate(manifest: &ComponentsManifest) -> Result<()> {
    if manifest.version != 1 || manifest.components.len() > MAX_COMPONENTS {
        anyhow::bail!("invalid components manifest version/count");
    }
    let mut ids = std::collections::BTreeSet::new();
    for component in &manifest.components {
        let import = Path::new(&component.import_path);
        if !valid_id(&component.id)
            || !ids.insert(&component.id)
            || component.name.trim().is_empty()
            || component.name.len() > 160
            || import.is_absolute()
            || import.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            anyhow::bail!("invalid component entry");
        }
        if component.modes.len() > 32
            || component.modes.iter().any(|mode| {
                !valid_id(&mode.id)
                    || (!mode.props.is_object() && !mode.props.is_null())
                    || serde_json::to_vec(&mode.props).map_or(true, |v| v.len() > 16 * 1024)
            })
        {
            anyhow::bail!("invalid component mode");
        }
    }
    if serde_json::to_vec(manifest)?.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!("components manifest exceeds 2 MiB");
    }
    Ok(())
}

fn paths(project_id: &str) -> Result<(PathBuf, PathBuf)> {
    let project = super::service::get_project(project_id)?
        .with_context(|| format!("design project not found: {project_id}"))?;
    let root = ha_core::paths::design_project_dir(&project.id)?;
    Ok((
        root.join("components.manifest.json"),
        root.join("components.manifest.draft.json"),
    ))
}

fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn read_one(path: &Path, draft: bool) -> Result<ManifestEnvelope> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            serde_json::to_vec(&ComponentsManifest::default())?
        }
        Err(e) => return Err(e).context("read components manifest"),
    };
    let manifest: ComponentsManifest = serde_json::from_slice(&bytes)?;
    validate(&manifest)?;
    Ok(ManifestEnvelope {
        manifest,
        hash: hash_bytes(&bytes),
        draft,
    })
}

pub fn get_published(project_id: &str) -> Result<ManifestEnvelope> {
    read_one(&paths(project_id)?.0, false)
}

pub fn get_draft(project_id: &str) -> Result<ManifestEnvelope> {
    let (published, draft) = paths(project_id)?;
    read_draft_from_paths(&published, &draft)
}

fn read_draft_from_paths(published: &Path, draft: &Path) -> Result<ManifestEnvelope> {
    if draft.exists() {
        read_one(draft, true)
    } else {
        let mut envelope = read_one(published, true)?;
        envelope.draft = true;
        Ok(envelope)
    }
}

pub fn save_draft(
    project_id: &str,
    expected_draft_hash: &str,
    manifest: ComponentsManifest,
) -> Result<ManifestEnvelope> {
    validate(&manifest)?;
    let (published, draft) = paths(project_id)?;
    save_draft_to_paths(&published, &draft, expected_draft_hash, manifest)
}

fn manifest_lock_path(published: &Path) -> PathBuf {
    published.with_extension("publish.lock")
}

fn acquire_manifest_lock(published: &Path) -> Result<std::fs::File> {
    ha_core::platform::try_acquire_exclusive_lock(&manifest_lock_path(published))?
        .ok_or_else(|| anyhow::anyhow!("components manifest update already in progress"))
}

fn save_draft_to_paths(
    published: &Path,
    draft: &Path,
    expected_draft_hash: &str,
    manifest: ComponentsManifest,
) -> Result<ManifestEnvelope> {
    let _manifest_guard = acquire_manifest_lock(published)?;
    let current = read_draft_from_paths(published, draft)?;
    if current.hash != expected_draft_hash {
        anyhow::bail!("stale components manifest: draft version changed");
    }
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    write_atomic(draft, &bytes)?;
    Ok(ManifestEnvelope {
        manifest,
        hash: hash_bytes(&bytes),
        draft: true,
    })
}

fn clear_synchronized_draft_with<F>(draft: &Path, published_bytes: &[u8], remove: F) -> Result<()>
where
    F: FnOnce(&Path) -> std::io::Result<()>,
{
    match remove(draft) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            let retained = std::fs::read(draft)
                .context("verify retained components draft after cleanup failure")?;
            if retained != published_bytes {
                return Err(error).context(
                    "remove published components draft; retained draft diverged from publication",
                );
            }
            // The canonical draft is byte-identical to the publication because
            // publication first synchronizes it under the same OS lock. It is
            // therefore safe for get_draft() to keep preferring this file even
            // when Windows temporarily refuses deletion.
            ha_core::app_warn!(
                "design",
                "components_manifest",
                "Unable to remove the published components draft; retaining the byte-identical synchronized copy: {}",
                error
            );
            Ok(())
        }
    }
}

fn publish_to_paths_with_remove<F>(
    published: &Path,
    draft: &Path,
    input: PublishManifestInput,
    remove: F,
) -> Result<ManifestEnvelope>
where
    F: FnOnce(&Path) -> std::io::Result<()>,
{
    validate(&input.manifest)?;
    let _manifest_guard = acquire_manifest_lock(published)?;
    let current = read_one(published, false)?;
    if current.hash != input.expected_published_hash {
        anyhow::bail!("stale components manifest: published version changed");
    }
    let current_draft = read_draft_from_paths(published, draft)?;
    if current_draft.hash != input.expected_draft_hash {
        anyhow::bail!("stale components manifest: draft version changed");
    }
    let bytes = serde_json::to_vec_pretty(&input.manifest)?;
    // Publishing also saves the submitted document as the canonical draft
    // first. If publication fails, the user's new draft remains retryable. If
    // later deletion fails, the surviving draft is already byte-identical to
    // the published document and cannot shadow it with stale content.
    write_atomic(draft, &bytes)?;
    write_atomic(published, &bytes)?;
    clear_synchronized_draft_with(draft, &bytes, remove)?;
    Ok(ManifestEnvelope {
        manifest: input.manifest,
        hash: hash_bytes(&bytes),
        draft: false,
    })
}

pub fn publish(input: PublishManifestInput) -> Result<ManifestEnvelope> {
    let (published, draft) = paths(&input.project_id)?;
    publish_to_paths_with_remove(&published, &draft, input, |path| std::fs::remove_file(path))
}

/// 只读候选扫描：不执行代码，只枚举绑定仓库内常见组件文件。
pub fn scan_candidates(project_id: &str) -> Result<Vec<ComponentEntry>> {
    let project = super::service::get_project(project_id)?
        .with_context(|| format!("design project not found: {project_id}"))?;
    let code_dir = super::service::resolve_code_dir(&project)
        .context("design project is not bound to a code directory")?;
    let root = std::fs::canonicalize(&code_dir)?;
    let mut out = Vec::new();
    scan_dir(&root, &root, 0, &mut out)?;
    out.sort_by(|a, b| a.import_path.cmp(&b.import_path));
    out.truncate(MAX_COMPONENTS);
    Ok(out)
}

fn scan_dir(root: &Path, dir: &Path, depth: usize, out: &mut Vec<ComponentEntry>) -> Result<()> {
    if depth > 8 || out.len() >= MAX_COMPONENTS {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            let name = entry.file_name();
            if matches!(
                name.to_str(),
                Some("node_modules" | ".git" | "dist" | "target")
            ) {
                continue;
            }
            scan_dir(root, &path, depth + 1, out)?;
            continue;
        }
        let Some(ext) = path.extension().and_then(|v| v.to_str()) else {
            continue;
        };
        if !matches!(ext, "tsx" | "jsx" | "vue" | "svelte") {
            continue;
        }
        let rel = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let stem = path
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or("Component");
        if !stem.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            continue;
        }
        out.push(ComponentEntry {
            id: blake3::hash(rel.as_bytes()).to_hex()[..16].to_string(),
            name: stem.to_string(),
            import_path: rel,
            export_name: Some(stem.to_string()),
            modes: Vec::new(),
        });
        if out.len() >= MAX_COMPONENTS {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_imports_and_non_object_props() {
        let manifest = ComponentsManifest {
            version: 1,
            components: vec![ComponentEntry {
                id: "button".into(),
                name: "Button".into(),
                import_path: "../Button.tsx".into(),
                export_name: None,
                modes: vec![],
            }],
        };
        assert!(validate(&manifest).is_err());
    }

    #[test]
    fn draft_save_uses_the_publication_lock() {
        let dir = tempfile::tempdir().unwrap();
        let published = dir.path().join("components.manifest.json");
        let draft = dir.path().join("components.manifest.draft.json");
        let original = read_draft_from_paths(&published, &draft).unwrap();
        let held = acquire_manifest_lock(&published).unwrap();

        assert!(save_draft_to_paths(
            &published,
            &draft,
            &original.hash,
            ComponentsManifest::default()
        )
        .is_err());
        assert!(!draft.exists());

        drop(held);
        assert!(save_draft_to_paths(
            &published,
            &draft,
            &original.hash,
            ComponentsManifest::default()
        )
        .is_ok());
        assert!(draft.exists());
    }

    #[test]
    fn stale_draft_save_is_rejected_without_overwriting_the_winner() {
        let dir = tempfile::tempdir().unwrap();
        let published = dir.path().join("components.manifest.json");
        let draft = dir.path().join("components.manifest.draft.json");
        let original = read_draft_from_paths(&published, &draft).unwrap();

        let mut winner = ComponentsManifest::default();
        winner.components.push(ComponentEntry {
            id: "winner".into(),
            name: "Winner".into(),
            import_path: "Winner.tsx".into(),
            export_name: Some("Winner".into()),
            modes: vec![],
        });
        let winner = save_draft_to_paths(&published, &draft, &original.hash, winner).unwrap();

        let error = save_draft_to_paths(
            &published,
            &draft,
            &original.hash,
            ComponentsManifest::default(),
        )
        .expect_err("stale draft save must fail closed");
        assert!(error.to_string().contains("draft version changed"));
        assert_eq!(read_one(&draft, true).unwrap().hash, winner.hash);
    }

    #[test]
    fn stale_publish_is_rejected_without_discarding_the_newer_draft() {
        let dir = tempfile::tempdir().unwrap();
        let published = dir.path().join("components.manifest.json");
        let draft = dir.path().join("components.manifest.draft.json");
        let published_before = read_one(&published, false).unwrap();
        let draft_before = read_draft_from_paths(&published, &draft).unwrap();

        let mut winner = ComponentsManifest::default();
        winner.components.push(ComponentEntry {
            id: "winner".into(),
            name: "Winner".into(),
            import_path: "Winner.tsx".into(),
            export_name: Some("Winner".into()),
            modes: vec![],
        });
        let winner = save_draft_to_paths(&published, &draft, &draft_before.hash, winner).unwrap();

        let error = publish_to_paths_with_remove(
            &published,
            &draft,
            PublishManifestInput {
                project_id: "test".into(),
                expected_published_hash: published_before.hash.clone(),
                expected_draft_hash: draft_before.hash,
                manifest: ComponentsManifest::default(),
            },
            |path| std::fs::remove_file(path),
        )
        .expect_err("publish with a stale draft hash must fail closed");

        assert!(error.to_string().contains("draft version changed"));
        assert_eq!(read_one(&draft, true).unwrap().hash, winner.hash);
        assert_eq!(
            read_one(&published, false).unwrap().hash,
            published_before.hash
        );
    }

    #[test]
    fn publish_retains_only_a_synchronized_draft_when_removal_fails() {
        let dir = tempfile::tempdir().unwrap();
        let published = dir.path().join("components.manifest.json");
        let draft = dir.path().join("components.manifest.draft.json");
        let published_before = read_one(&published, false).unwrap();
        let draft_before = read_draft_from_paths(&published, &draft).unwrap();
        let mut manifest = ComponentsManifest::default();
        manifest.components.push(ComponentEntry {
            id: "published".into(),
            name: "Published".into(),
            import_path: "Published.tsx".into(),
            export_name: Some("Published".into()),
            modes: vec![],
        });

        let saved = publish_to_paths_with_remove(
            &published,
            &draft,
            PublishManifestInput {
                project_id: "test".into(),
                expected_published_hash: published_before.hash,
                expected_draft_hash: draft_before.hash,
                manifest,
            },
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "synthetic removal denial",
                ))
            },
        )
        .expect("byte-identical retained draft is a reconciled success");

        assert_eq!(read_one(&published, false).unwrap().hash, saved.hash);
        assert_eq!(read_one(&draft, true).unwrap().hash, saved.hash);
    }
}
