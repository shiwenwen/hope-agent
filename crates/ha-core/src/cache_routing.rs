//! Persistent, installation-local keying for provider prompt-cache routing.
//!
//! Routing identifiers leave the process in provider request bodies, so they
//! must not be raw hashes of user-authored prompt text. A dedicated key keeps
//! equal stable prefixes routable across restarts on this installation while
//! preventing offline dictionary matching and avoiding reuse of the shorter-
//! lived telemetry fingerprint key.

use anyhow::{Context, Result};
use std::{
    path::Path,
    sync::OnceLock,
    time::{Duration, Instant},
};

const KEY_FILE_NAME: &str = "prompt-cache-routing-v1.key";
const LOCK_FILE_NAME: &str = "prompt-cache-routing-v1.lock";
const KEY_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(2);
const INITIAL_LOCK_RETRY_DELAY: Duration = Duration::from_millis(5);
const MAX_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

static ROUTING_KEY: OnceLock<[u8; 32]> = OnceLock::new();

pub(crate) fn init() -> Result<()> {
    if ROUTING_KEY.get().is_some() {
        return Ok(());
    }
    match load_or_create_key() {
        Ok(key) => {
            let _ = ROUTING_KEY.set(key);
            Ok(())
        }
        Err(error) => {
            // Availability fallback only: the key stays process-local, so
            // cache affinity may reset on restart but prompt material is never
            // exposed through a raw digest.
            let _ = ROUTING_KEY.set(rand::random());
            Err(error)
        }
    }
}

fn load_or_create_key() -> Result<[u8; 32]> {
    let directory = crate::paths::credentials_dir()?;
    load_or_create_key_in(&directory)
}

fn load_or_create_key_in(directory: &Path) -> Result<[u8; 32]> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("create credentials directory {}", directory.display()))?;
    let path = directory.join(KEY_FILE_NAME);
    let lock_path = directory.join(LOCK_FILE_NAME);
    let deadline = Instant::now() + KEY_PUBLICATION_TIMEOUT;
    let mut retry_delay = INITIAL_LOCK_RETRY_DELAY;

    loop {
        // Publication is atomic, so a visible file is always complete. Re-read
        // before every lock attempt: the current publisher may have finished
        // while this process was backing off.
        if let Some(key) = read_key(&path)? {
            return Ok(key);
        }

        match crate::platform::try_acquire_exclusive_lock(&lock_path)
            .with_context(|| format!("lock prompt-cache routing key {}", lock_path.display()))?
        {
            Some(_lock) => {
                // The previous publisher may have committed immediately before
                // releasing the lock. Never replace its installation key.
                if let Some(key) = read_key(&path)? {
                    return Ok(key);
                }
                let key: [u8; 32] = rand::random();
                crate::platform::write_secure_file(&path, &key).with_context(|| {
                    format!("write prompt-cache routing key {}", path.display())
                })?;
                return read_key(&path)?.context("prompt-cache routing key write was not readable");
            }
            None => {
                let now = Instant::now();
                if now >= deadline {
                    // Close the small race where the publisher committed after
                    // our first read but before the final timeout decision.
                    if let Some(key) = read_key(&path)? {
                        return Ok(key);
                    }
                    anyhow::bail!("timed out waiting for prompt-cache routing key publication");
                }

                std::thread::sleep(retry_delay.min(deadline.saturating_duration_since(now)));
                retry_delay = retry_delay.saturating_mul(2).min(MAX_LOCK_RETRY_DELAY);
            }
        }
    }
}

fn read_key(path: &std::path::Path) -> Result<Option<[u8; 32]>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("prompt-cache routing key has an invalid length"))?;
    Ok(Some(key))
}

pub(crate) fn keyed_digest<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> blake3::Hash {
    let key = ROUTING_KEY.get_or_init(rand::random);
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(b"hope-agent:prompt-cache-routing:v1\0");
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize()
}

/// Content-free identifier for diagnostics that need to correlate repeated
/// provider failures without persisting provider-controlled response text.
/// The installation-local key prevents offline matching of short secrets or
/// user prompt fragments that a provider may echo in an error body.
pub(crate) fn audit_fingerprint(domain: &str, value: &[u8]) -> String {
    keyed_digest([b"audit-fingerprint:v1".as_slice(), domain.as_bytes(), value])
        .to_hex()
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
        time::Duration,
    };

    use super::{load_or_create_key_in, KEY_FILE_NAME, LOCK_FILE_NAME};

    #[test]
    fn waits_for_a_slow_publisher_and_reuses_its_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let directory = temp.path().to_path_buf();
        let lock_path = directory.join(LOCK_FILE_NAME);
        let publisher_lock = crate::platform::try_acquire_exclusive_lock(&lock_path)
            .expect("acquire publisher lock")
            .expect("publisher owns lock");
        let expected = [0x5a; 32];
        let start = Arc::new(Barrier::new(2));
        let worker_start = Arc::clone(&start);
        let worker_directory = directory.clone();
        let worker = thread::spawn(move || {
            worker_start.wait();
            load_or_create_key_in(&worker_directory)
        });

        start.wait();
        thread::sleep(Duration::from_millis(75));
        crate::platform::write_secure_file(&directory.join(KEY_FILE_NAME), &expected)
            .expect("publish key");
        drop(publisher_lock);

        assert_eq!(
            worker.join().expect("worker join").expect("load key"),
            expected
        );
    }

    #[test]
    fn retries_the_lock_and_takes_over_after_publisher_exit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let directory = temp.path().to_path_buf();
        let lock_path = directory.join(LOCK_FILE_NAME);
        let publisher_lock = crate::platform::try_acquire_exclusive_lock(&lock_path)
            .expect("acquire publisher lock")
            .expect("publisher owns lock");
        let start = Arc::new(Barrier::new(2));
        let worker_start = Arc::clone(&start);
        let worker_directory = directory.clone();
        let worker = thread::spawn(move || {
            worker_start.wait();
            load_or_create_key_in(&worker_directory)
        });

        start.wait();
        thread::sleep(Duration::from_millis(75));
        drop(publisher_lock);

        let key = worker.join().expect("worker join").expect("create key");
        assert_eq!(
            std::fs::read(directory.join(KEY_FILE_NAME)).expect("read published key"),
            key
        );
    }
}
