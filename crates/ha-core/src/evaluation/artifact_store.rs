use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct EvalArtifactStore {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct StoredEvalArtifact {
    pub sha256: String,
    pub size_bytes: u64,
    pub path: PathBuf,
}

impl EvalArtifactStore {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)
            .with_context(|| format!("creating evaluation artifact store {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn default_store() -> Result<Self> {
        Self::open(crate::paths::eval_artifacts_dir()?)
    }

    pub fn put_bytes(&self, bytes: &[u8]) -> Result<StoredEvalArtifact> {
        let sha256 = hex_digest(bytes);
        let path = self.path_for(&sha256)?;
        if path.exists() {
            let existing = fs::read(&path)?;
            if hex_digest(&existing) != sha256 {
                bail!("content-addressed evaluation artifact is corrupt");
            }
        } else {
            crate::platform::write_atomic_create_new(&path, bytes).or_else(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(error)
                }
            })?;
        }
        Ok(StoredEvalArtifact {
            sha256,
            size_bytes: bytes.len() as u64,
            path,
        })
    }

    pub fn put_file(&self, source: &Path, max_bytes: u64) -> Result<StoredEvalArtifact> {
        let metadata = fs::symlink_metadata(source)
            .with_context(|| format!("reading evaluation artifact {}", source.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("evaluation artifact source must be a regular non-symlink file");
        }
        if metadata.len() > max_bytes {
            bail!("evaluation artifact exceeds the configured size limit");
        }
        self.put_bytes(&fs::read(source)?)
    }

    pub fn read(&self, sha256: &str, max_bytes: u64) -> Result<Vec<u8>> {
        let path = self.path_for(sha256)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
            bail!("evaluation artifact is missing, unsafe, or too large");
        }
        let bytes = fs::read(path)?;
        if hex_digest(&bytes) != sha256 {
            bail!("evaluation artifact digest verification failed");
        }
        Ok(bytes)
    }

    pub fn path_for(&self, sha256: &str) -> Result<PathBuf> {
        validate_sha256(sha256)?;
        Ok(self.root.join(&sha256[..2]).join(&sha256[2..]))
    }

    pub fn remove_unreferenced(&self, sha256: &str) -> Result<()> {
        let path = self.path_for(sha256)?;
        if path.is_file() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Apply database-owned retention decisions. Protected or pinned objects
    /// never appear in the candidate set, and the DB row is removed only
    /// after the content-addressed file deletion succeeds.
    pub fn prune_expired(&self, repository: &super::EvalRepository) -> Result<usize> {
        let mut removed = 0usize;
        for sha256 in repository.expired_artifact_sha256s()? {
            self.remove_unreferenced(&sha256)?;
            if repository.forget_collectable_artifact(&sha256)? {
                removed = removed.saturating_add(1);
            }
        }
        Ok(removed)
    }
}

pub fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("invalid lowercase SHA-256 digest");
    }
    Ok(())
}

pub fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_addressing_deduplicates_and_detects_bad_ids() {
        let temp = tempfile::tempdir().unwrap();
        let store = EvalArtifactStore::open(temp.path().to_path_buf()).unwrap();
        let first = store.put_bytes(b"evidence").unwrap();
        let second = store.put_bytes(b"evidence").unwrap();
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(store.read(&first.sha256, 100).unwrap(), b"evidence");
        assert!(store.path_for("../escape").is_err());
    }
}
