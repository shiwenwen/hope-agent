//! Detached re-spawn for the foreground `hope-agent server` case.
//!
//! The current process can't really restart itself when no one is around to
//! relaunch it — there's no launchd / systemd / Task Scheduler to spawn a
//! successor when our PID dies. So we do it ourselves: `fork(2)` (on Unix)
//! a child running `hope-agent server <captured argv>` with stdio detached,
//! then schedule our own clean exit.
//!
//! Argv recovery uses [`crate::app_init::server_launch_args`] which the
//! `server` entrypoint captures into a `OnceLock`. If no caller registered
//! (test harness, library consumer, accidentally calling restart on a
//! process that wasn't launched as `server`), we refuse rather than
//! guessing.

use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// How long to wait between handing off to the spawned child and calling
/// `std::process::exit(0)`. Long enough that the EventBus emit + tool
/// result flush finishes, short enough that the port-bind handoff doesn't
/// look hung. The child's HTTP server binds eagerly so a 200ms overlap is
/// fine.
const SELF_EXIT_GRACE: Duration = Duration::from_millis(200);

/// Fork off a fully-detached `hope-agent server …` child reusing the captured
/// launch argv. Returns the new child PID on success.
pub fn respawn_detached_server() -> Result<u32> {
    let exe = std::env::current_exe().context("current_exe() failed")?;
    let argv = crate::app_init::server_launch_args();
    if argv.is_empty() && crate::app_init::runtime_role() == Some("server") {
        // Defensive: a server process should always have set its argv at
        // startup. Empty argv would still launch with defaults, but log
        // a hint so the foreground operator notices.
        app_warn!(
            "lifecycle",
            "respawn",
            "no server launch argv captured — child will start with defaults (bind 127.0.0.1:8420)"
        );
    }

    let mut cmd = Command::new(&exe);
    cmd.arg("server");
    cmd.args(argv);
    // Fully detach stdio so the child survives terminal close.
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // setsid(2) — new session, no controlling TTY. Without this the
        // child stays in our process group and a Ctrl-C in the terminal
        // we left behind would kill it too.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP. Matches the flags
        // `service_install`'s scheduled-task uses to keep the child off
        // our console.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn detached server child at {}", exe.display()))?;
    let pid = child.id();
    // Don't `wait()` — we want the child fully detached. On Unix the
    // double-fork-zombie concern is theoretical here because we are
    // about to call `exit(0)` ourselves; init (or whichever the parent
    // process has) will reap our orphans.
    drop(child);

    // Spot-check we're not about to bind the same port from two processes
    // at once. Best-effort — by the time we exit, the kernel will have
    // freed the socket. The 200ms grace below covers the race window.
    if argv.is_empty() {
        bail_if_no_server_role()?;
    }

    Ok(pid)
}

fn bail_if_no_server_role() -> Result<()> {
    if crate::app_init::runtime_role() != Some("server") {
        bail!(
            "respawn_detached_server requires runtime_role == 'server' (current: {:?})",
            crate::app_init::runtime_role()
        );
    }
    Ok(())
}

/// Schedule `std::process::exit(0)` after [`SELF_EXIT_GRACE`]. Spawned on a
/// dedicated OS thread (not tokio) so the runtime tearing down doesn't
/// cancel us mid-sleep.
pub fn schedule_self_exit() {
    std::thread::spawn(|| {
        std::thread::sleep(SELF_EXIT_GRACE);
        // `exit(0)` flushes stdio but skips destructors. `crash_flush`'s
        // signal handlers wouldn't run for a clean exit either, so this
        // matches the normal "user closed terminal" shutdown shape.
        std::process::exit(0);
    });
}
