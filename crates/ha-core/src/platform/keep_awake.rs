//! Keep the host awake (prevent system idle sleep) while the user setting is on.
//!
//! Cross-platform, dependency-light. Every backend is bound to this process's
//! lifetime so a crash or hard-quit can never strand the assertion (keeping the
//! host awake forever):
//! - **macOS**: `caffeinate -i -w <pid>` holds an idle-sleep power assertion and
//!   self-exits when our pid dies. `-i` inhibits only *system* idle sleep, so
//!   the display may still sleep.
//! - **Linux**: `systemd-inhibit --what=sleep:idle … tail --pid=<pid> -f
//!   /dev/null` holds a logind inhibitor lock for as long as our process lives
//!   (the wrapped `tail` exits when our pid dies, releasing the lock). No-op
//!   (logged) on hosts without `systemd-inhibit`.
//! - **Windows**: a dedicated thread calls `SetThreadExecutionState(ES_CONTINUOUS
//!   | ES_SYSTEM_REQUIRED)`. That flag is process-bound (auto-cleared on exit)
//!   and thread-affine, so we park the thread and clear + return on release.
//!
//! [`apply`] is idempotent: it acquires when turning on (and not already
//! active), releases when turning off, and no-ops when already in the requested
//! state. Safe to call on startup and on every `config:changed`.

use std::sync::Mutex;

/// Process-wide assertion holder. `None` = sleep prevention is off.
static STATE: Mutex<Option<Guard>> = Mutex::new(None);

/// Acquire or release the OS sleep assertion to match `enabled`.
///
/// Idempotent — calling repeatedly with the same value is a cheap no-op. Only
/// the assertion-holding (primary) process should drive this; see
/// `app_init::spawn_keep_awake_listener`.
pub fn apply(enabled: bool) {
    let mut state = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if enabled == state.is_some() {
        return; // already in the requested state
    }
    if enabled {
        match Guard::acquire() {
            Some(guard) => {
                *state = Some(guard);
                crate::app_info!("platform", "keep_awake", "system sleep prevention enabled");
            }
            None => crate::app_warn!(
                "platform",
                "keep_awake",
                "failed to acquire sleep-prevention assertion"
            ),
        }
    } else if let Some(guard) = state.take() {
        guard.release();
        crate::app_info!("platform", "keep_awake", "system sleep prevention disabled");
    }
}

// ── Unix (macOS + Linux/BSD) — child process holds the assertion ──────────

#[cfg(unix)]
struct Guard {
    child: std::process::Child,
}

#[cfg(target_os = "macos")]
const KEEP_AWAKE_BIN: &str = "caffeinate";
#[cfg(all(unix, not(target_os = "macos")))]
const KEEP_AWAKE_BIN: &str = "systemd-inhibit";

#[cfg(unix)]
impl Guard {
    fn acquire() -> Option<Self> {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let pid = std::process::id().to_string();
        let mut cmd = Command::new(KEEP_AWAKE_BIN);

        #[cfg(target_os = "macos")]
        cmd.arg("-i").arg("-w").arg(&pid);

        #[cfg(all(unix, not(target_os = "macos")))]
        cmd.arg("--what=sleep:idle")
            .arg("--who=Hope Agent")
            .arg("--why=Keep system awake (user setting)")
            .arg("--mode=block")
            // Inhibited command: blocks until our pid dies, then exits so the
            // logind lock is released even if we never call `release`.
            .arg("tail")
            .arg(format!("--pid={pid}"))
            .arg("-f")
            .arg("/dev/null");

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Own process group so `release` can signal the whole tree —
            // `systemd-inhibit` spawns a `tail` child that must die too.
            .process_group(0);

        match cmd.spawn() {
            Ok(child) => Some(Self { child }),
            Err(e) => {
                crate::app_warn!(
                    "platform",
                    "keep_awake",
                    "spawn `{}` failed: {}",
                    KEEP_AWAKE_BIN,
                    e
                );
                None
            }
        }
    }

    fn release(mut self) {
        // SIGTERM the child's process group (group leader + any children).
        let pgid = self.child.id() as i32;
        // SAFETY: signalling a process group we created; harmless if already gone.
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
        let _ = self.child.wait();
    }
}

// ── Windows — dedicated thread holds ES_CONTINUOUS | ES_SYSTEM_REQUIRED ────

#[cfg(windows)]
struct Guard {
    stop: std::sync::mpsc::Sender<()>,
    handle: std::thread::JoinHandle<()>,
}

#[cfg(windows)]
mod winapi {
    pub const ES_CONTINUOUS: u32 = 0x8000_0000;
    pub const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn SetThreadExecutionState(es_flags: u32) -> u32;
    }
}

#[cfg(windows)]
impl Guard {
    fn acquire() -> Option<Self> {
        let (stop, rx) = std::sync::mpsc::channel::<()>();
        let handle = std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || {
                // Hold the assertion: the execution-state flag is thread-affine
                // and persists until this thread clears it or exits.
                // SAFETY: SetThreadExecutionState takes a flag bitmask; no ptrs.
                unsafe {
                    winapi::SetThreadExecutionState(
                        winapi::ES_CONTINUOUS | winapi::ES_SYSTEM_REQUIRED,
                    );
                }
                // Block until release() signals (or the sender is dropped).
                let _ = rx.recv();
                // SAFETY: clear the continuous assertion before the thread exits.
                unsafe {
                    winapi::SetThreadExecutionState(winapi::ES_CONTINUOUS);
                }
            })
            .ok()?;
        Some(Self { stop, handle })
    }

    fn release(self) {
        let _ = self.stop.send(());
        let _ = self.handle.join();
    }
}
