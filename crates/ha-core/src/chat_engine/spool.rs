//! Emergency durable spool used when the SQLite journal is temporarily
//! unavailable. Frames are append-only, checksummed, and fsynced before their
//! corresponding events may be shown.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::session::JournalBatch;

const FRAME_VERSION: u8 = 1;
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpoolFrame {
    version: u8,
    checksum: String,
    batch: JournalBatch,
}

/// Result of scanning an append-only spool. A torn/corrupt tail never hides
/// the complete, checksummed prefix that precedes it. Startup recovery imports
/// `batches` and records `integrity_error` on the recovered turn; a live final
/// commit treats any integrity error as fatal and leaves the file in place.
#[derive(Debug)]
pub struct SpoolReadResult {
    pub batches: Vec<JournalBatch>,
    pub integrity_error: Option<String>,
}

fn ensure_secure_dir() -> Result<PathBuf> {
    let root = crate::paths::root_dir()?;
    let dir = crate::paths::stream_spool_dir()?;
    if let Ok(meta) = std::fs::symlink_metadata(&dir) {
        if meta.file_type().is_symlink() {
            anyhow::bail!("stream spool directory may not be a symlink");
        }
    } else {
        std::fs::create_dir_all(&dir)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let canonical_root = root.canonicalize()?;
    let canonical_dir = dir.canonicalize()?;
    if !canonical_dir.starts_with(&canonical_root) {
        anyhow::bail!("stream spool directory escapes the data root");
    }
    Ok(canonical_dir)
}

fn checked_path(run_id: &str) -> Result<PathBuf> {
    let expected = crate::paths::stream_spool_path(run_id)?;
    let dir = ensure_secure_dir()?;
    let file_name = expected
        .file_name()
        .context("stream spool path has no file name")?;
    let path = dir.join(file_name);
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if meta.file_type().is_symlink() {
            anyhow::bail!("stream spool file may not be a symlink");
        }
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(&dir) {
            anyhow::bail!("stream spool file escapes the spool directory");
        }
    }
    Ok(path)
}

fn open_append(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true).read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

fn open_read(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(Into::into)
}

pub fn append_batch(batch: &JournalBatch) -> Result<()> {
    let batch_json = serde_json::to_vec(batch)?;
    let frame = SpoolFrame {
        version: FRAME_VERSION,
        checksum: blake3::hash(&batch_json).to_hex().to_string(),
        batch: batch.clone(),
    };
    let encoded = serde_json::to_vec(&frame)?;
    if encoded.len() > MAX_FRAME_BYTES {
        anyhow::bail!("stream spool frame exceeds maximum size");
    }
    let path = checked_path(&batch.run_id)?;
    #[cfg(unix)]
    let existed = path.exists();
    let mut file = open_append(&path)?;
    file.write_all(&(encoded.len() as u64).to_le_bytes())?;
    file.write_all(&encoded)?;
    file.sync_data()?;
    #[cfg(unix)]
    if !existed {
        // `sync_data` makes frame bytes durable but does not guarantee the new
        // directory entry survives sudden power loss. Sync the 0700 parent on
        // first creation before acknowledging the batch to the UI.
        let parent = path.parent().context("stream spool path has no parent")?;
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

pub fn read_batches(run_id: &str) -> Result<SpoolReadResult> {
    let path = checked_path(run_id)?;
    if !path.exists() {
        return Ok(SpoolReadResult {
            batches: Vec::new(),
            integrity_error: None,
        });
    }
    let mut file = open_read(&path)?;
    let mut out = Vec::new();
    let mut frame_no = 0u64;
    loop {
        let frame_start = file.stream_position()?;
        let mut len_buf = [0u8; 8];
        match file.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                // A clean EOF reads zero header bytes; a short header is a
                // torn tail. `stream_position` lets us distinguish them
                // without ever logging frame payloads.
                let length = file.metadata()?.len();
                let integrity_error = (frame_start != length)
                    .then(|| format!("truncated stream spool header at frame {}", frame_no + 1));
                return Ok(SpoolReadResult {
                    batches: out,
                    integrity_error,
                });
            }
            Err(error) => return Err(error.into()),
        }
        frame_no = frame_no.saturating_add(1);
        let len = u64::from_le_bytes(len_buf) as usize;
        if len == 0 || len > MAX_FRAME_BYTES {
            return Ok(SpoolReadResult {
                batches: out,
                integrity_error: Some(format!(
                    "invalid stream spool frame length at frame {frame_no}"
                )),
            });
        }
        let mut encoded = vec![0u8; len];
        if let Err(error) = file.read_exact(&mut encoded) {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                return Ok(SpoolReadResult {
                    batches: out,
                    integrity_error: Some(format!(
                        "truncated stream spool payload at frame {frame_no}"
                    )),
                });
            }
            return Err(error.into());
        }
        let frame: SpoolFrame = match serde_json::from_slice(&encoded) {
            Ok(frame) => frame,
            Err(_) => {
                return Ok(SpoolReadResult {
                    batches: out,
                    integrity_error: Some(format!(
                        "invalid stream spool payload at frame {frame_no}"
                    )),
                });
            }
        };
        if frame.version != FRAME_VERSION {
            return Ok(SpoolReadResult {
                batches: out,
                integrity_error: Some(format!(
                    "unsupported stream spool frame version at frame {frame_no}"
                )),
            });
        }
        let batch_json = serde_json::to_vec(&frame.batch)?;
        if blake3::hash(&batch_json).to_hex().as_str() != frame.checksum {
            return Ok(SpoolReadResult {
                batches: out,
                integrity_error: Some(format!(
                    "stream spool checksum mismatch at frame {frame_no}"
                )),
            });
        }
        out.push(frame.batch);
    }
}

pub fn remove(run_id: &str) -> Result<()> {
    let path = checked_path(run_id)?;
    if path.exists() {
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            anyhow::bail!("stream spool removal target is not a regular file");
        }
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Preserve a damaged spool verbatim after its valid prefix was recovered.
/// Quarantined files are ignored by normal import and removed by the same
/// 24-hour retention loop as terminal journal rows.
pub fn quarantine(run_id: &str) -> Result<()> {
    let path = checked_path(run_id)?;
    if !path.exists() {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        anyhow::bail!("stream spool quarantine target is not a regular file");
    }
    let dir = ensure_secure_dir()?;
    let timestamp = chrono::Utc::now().timestamp_millis();
    let target = dir.join(format!("{run_id}.corrupt.{timestamp}.log"));
    std::fs::rename(path, target)?;
    Ok(())
}

pub fn gc_quarantined(retention: std::time::Duration) -> Result<usize> {
    let dir = ensure_secure_dir()?;
    let now = std::time::SystemTime::now();
    let mut removed = 0usize;
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || !file_type.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        let Some((run_id, suffix)) = name.split_once(".corrupt.") else {
            continue;
        };
        if !suffix.ends_with(".log") || uuid::Uuid::parse_str(run_id).is_err() {
            continue;
        }
        let metadata = entry.metadata()?;
        let old_enough = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= retention);
        if !old_enough {
            continue;
        }
        let path = entry.path();
        let current = std::fs::symlink_metadata(&path)?;
        if current.file_type().is_symlink() || !current.file_type().is_file() {
            continue;
        }
        std::fs::remove_file(path)?;
        removed = removed.saturating_add(1);
    }
    Ok(removed)
}

pub fn list_run_ids() -> Result<Vec<String>> {
    let dir = ensure_secure_dir()?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() || !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(run_id) = name.strip_suffix(".log") else {
            continue;
        };
        if uuid::Uuid::parse_str(run_id).is_ok() {
            out.push(run_id.to_string());
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::JournalEvent;

    fn batch(run_id: &str, block_no: u64, seq: u64) -> JournalBatch {
        JournalBatch {
            run_id: run_id.to_string(),
            attempt_no: 1,
            block_no,
            seq_start: seq,
            seq_end: seq,
            events: vec![JournalEvent::single(
                seq,
                serde_json::json!({"type":"text_delta","content":seq.to_string()}).to_string(),
            )],
        }
    }

    #[test]
    fn spool_round_trip_preserves_checksums_and_permissions() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let run_id = uuid::Uuid::new_v4().to_string();
            append_batch(&batch(&run_id, 1, 1)).expect("first frame");
            append_batch(&batch(&run_id, 2, 2)).expect("second frame");
            let read = read_batches(&run_id).expect("read spool");
            assert!(read.integrity_error.is_none());
            assert_eq!(read.batches.len(), 2);
            assert_eq!(read.batches[0].seq_start, 1);
            assert_eq!(read.batches[1].seq_end, 2);

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let dir_mode = std::fs::metadata(crate::paths::stream_spool_dir().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                let file_mode =
                    std::fs::metadata(crate::paths::stream_spool_path(&run_id).unwrap())
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777;
                assert_eq!(dir_mode, 0o700);
                assert_eq!(file_mode, 0o600);
            }
        });
    }

    #[test]
    fn torn_spool_tail_keeps_largest_valid_prefix() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let run_id = uuid::Uuid::new_v4().to_string();
            append_batch(&batch(&run_id, 1, 1)).expect("first frame");
            let path = crate::paths::stream_spool_path(&run_id).expect("path");
            let mut file = OpenOptions::new().append(true).open(path).expect("append");
            file.write_all(&64u64.to_le_bytes()).expect("length");
            file.write_all(b"torn").expect("tail");
            file.sync_data().expect("sync tail");

            let read = read_batches(&run_id).expect("read torn spool");
            assert_eq!(read.batches.len(), 1);
            assert!(read
                .integrity_error
                .as_deref()
                .is_some_and(|error| error.contains("truncated")));
        });
    }

    #[cfg(unix)]
    #[test]
    fn spool_rejects_symlink_target() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            std::fs::create_dir_all(crate::paths::stream_spool_dir().unwrap()).expect("spool dir");
            let run_id = uuid::Uuid::new_v4().to_string();
            let outside = root.path().join("outside.log");
            std::fs::write(&outside, b"secret").expect("outside");
            symlink(&outside, crate::paths::stream_spool_path(&run_id).unwrap()).expect("symlink");
            assert!(append_batch(&batch(&run_id, 1, 1)).is_err());
            assert_eq!(std::fs::read(outside).unwrap(), b"secret");
        });
    }
}
