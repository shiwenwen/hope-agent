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

struct RoutingKeyState {
    key: [u8; 32],
    initialization_error: Option<String>,
}

impl RoutingKeyState {
    fn initialize(load: impl FnOnce() -> Result<[u8; 32]>) -> Self {
        match load() {
            Ok(key) => Self {
                key,
                initialization_error: None,
            },
            Err(error) => Self {
                // Availability fallback only: the key stays process-local, so
                // cache affinity may reset on restart but prompt material is
                // never exposed through a raw digest.
                key: rand::random(),
                initialization_error: Some(format!("{error:#}")),
            },
        }
    }

    fn initialization_result(&self) -> Result<()> {
        match &self.initialization_error {
            Some(error) => Err(anyhow::anyhow!(error.clone())),
            None => Ok(()),
        }
    }
}

static ROUTING_STATE: OnceLock<RoutingKeyState> = OnceLock::new();

fn routing_state_with(
    state: &OnceLock<RoutingKeyState>,
    load: impl FnOnce() -> Result<[u8; 32]>,
) -> &RoutingKeyState {
    state.get_or_init(|| RoutingKeyState::initialize(load))
}

fn routing_state() -> &'static RoutingKeyState {
    routing_state_with(&ROUTING_STATE, load_or_create_key)
}

pub(crate) fn init() -> Result<()> {
    routing_state().initialization_result()
}

fn load_or_create_key() -> Result<[u8; 32]> {
    let directory = crate::paths::credentials_dir()?;
    load_or_create_key_in(&directory)
}

fn load_or_create_key_in(directory: &Path) -> Result<[u8; 32]> {
    load_or_create_key_in_with_writer(directory, crate::platform::write_secure_file)
}

fn load_or_create_key_in_with_writer(
    directory: &Path,
    mut write_key: impl FnMut(&Path, &[u8]) -> std::io::Result<()>,
) -> Result<[u8; 32]> {
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
                if let Err(write_error) = write_key(&path, &key) {
                    // On Unix the atomic rename may have succeeded before a
                    // parent-directory fsync reports an error. Re-read the
                    // exact published path before falling back: another
                    // process will observe that key, so this process must use
                    // it too whenever it is a valid complete key.
                    match read_key(&path) {
                        Ok(Some(published_key)) => return Ok(published_key),
                        Ok(None) => {
                            return Err(write_error).with_context(|| {
                                format!("write prompt-cache routing key {}", path.display())
                            });
                        }
                        Err(read_error) => {
                            return Err(anyhow::anyhow!(
                                "write prompt-cache routing key {} failed: {}; reread failed: {}",
                                path.display(),
                                write_error,
                                read_error
                            ));
                        }
                    }
                }
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

fn keyed_digest_with<'a>(
    state: &OnceLock<RoutingKeyState>,
    load: impl FnOnce() -> Result<[u8; 32]>,
    parts: impl IntoIterator<Item = &'a [u8]>,
) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new_keyed(&routing_state_with(state, load).key);
    hasher.update(b"hope-agent:prompt-cache-routing:v1\0");
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize()
}

pub(crate) fn keyed_digest<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> blake3::Hash {
    keyed_digest_with(&ROUTING_STATE, load_or_create_key, parts)
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
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Barrier, Mutex, OnceLock,
        },
        thread,
        time::Duration,
    };

    use super::{
        keyed_digest_with, load_or_create_key_in, load_or_create_key_in_with_writer,
        routing_state_with, KEY_FILE_NAME, LOCK_FILE_NAME,
    };

    #[test]
    fn process_state_runs_only_one_initializer_under_concurrency() {
        let state = Arc::new(OnceLock::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(9));
        let expected = [0x31; 32];
        let mut workers = Vec::new();

        for _ in 0..8 {
            let state = Arc::clone(&state);
            let calls = Arc::clone(&calls);
            let start = Arc::clone(&start);
            workers.push(thread::spawn(move || {
                start.wait();
                routing_state_with(&state, || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    Ok(expected)
                })
                .key
            }));
        }

        start.wait();
        for worker in workers {
            assert_eq!(worker.join().expect("worker join"), expected);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn first_consumer_freezes_persistent_key_and_success_status() {
        let state = OnceLock::new();
        let expected = [0x42; 32];
        let first = routing_state_with(&state, || Ok(expected));
        assert_eq!(first.key, expected);
        assert!(first.initialization_result().is_ok());

        let reused = routing_state_with(&state, || -> anyhow::Result<[u8; 32]> {
            panic!("initializer must not run twice")
        });
        assert_eq!(reused.key, expected);
        assert!(reused.initialization_result().is_ok());
    }

    #[test]
    fn failed_first_consumer_freezes_one_fallback_and_error_status() {
        let state = OnceLock::new();
        let first = routing_state_with(&state, || anyhow::bail!("persistent key unavailable"));
        let fallback = first.key;
        assert!(first
            .initialization_result()
            .expect_err("fallback status")
            .to_string()
            .contains("persistent key unavailable"));

        let reused = routing_state_with(&state, || Ok([0x77; 32]));
        assert_eq!(reused.key, fallback);
        assert!(reused.initialization_result().is_err());
    }

    #[test]
    fn digest_before_explicit_init_runs_persistent_initializer() {
        let temp = tempfile::tempdir().expect("tempdir");
        let directory = temp.path().to_path_buf();
        let state = OnceLock::new();
        let parts = [b"stable prompt".as_slice(), b"tool schemas".as_slice()];

        let first = keyed_digest_with(&state, || load_or_create_key_in(&directory), parts);
        let persisted = std::fs::read(directory.join(KEY_FILE_NAME)).expect("persistent key");
        assert_eq!(persisted.len(), 32);
        let initialized = state.get().expect("digest initialized state");
        assert_eq!(initialized.key.as_slice(), persisted);
        assert!(initialized.initialization_result().is_ok());

        let second = keyed_digest_with(
            &state,
            || -> anyhow::Result<[u8; 32]> { panic!("initializer must not run twice") },
            parts,
        );
        assert_eq!(first, second);
    }

    #[test]
    fn adopts_key_published_before_writer_reports_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let published = Arc::new(Mutex::new(None));
        let observed = Arc::clone(&published);

        let key = load_or_create_key_in_with_writer(temp.path(), move |path, bytes| {
            std::fs::write(path, bytes)?;
            *observed.lock().expect("published key lock") = Some(bytes.to_vec());
            Err(std::io::Error::other(
                "simulated parent-directory fsync failure",
            ))
        })
        .expect("visible complete key must be adopted");

        assert_eq!(
            key.as_slice(),
            published
                .lock()
                .expect("published key lock")
                .as_deref()
                .expect("writer observed key")
        );
        assert_eq!(
            std::fs::read(temp.path().join(KEY_FILE_NAME)).expect("read published key"),
            key
        );
    }

    #[test]
    fn rejects_invalid_key_left_by_failed_writer() {
        let temp = tempfile::tempdir().expect("tempdir");
        let error = load_or_create_key_in_with_writer(temp.path(), |path, _bytes| {
            std::fs::write(path, [0x7a; 31])?;
            Err(std::io::Error::other(
                "simulated parent-directory fsync failure",
            ))
        })
        .expect_err("partial key must not be adopted");

        assert!(error.to_string().contains("reread failed"));
        assert!(format!("{error:#}").contains("invalid length"));
    }

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
