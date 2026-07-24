//! `Stop` block-to-continue, end to end on the live conversation path.
//!
//! The official Claude Code `Stop` protocol inverts `block`: the action being
//! vetoed IS stopping, so `exit 2` means "prevent stopping, run another turn"
//! and stderr is the feedback injected before it. `hooks::fire_stop` implements
//! that by re-driving the session through the subagent injection pipeline —
//! which makes it the only hook path that can *resurrect* a finished turn, and
//! therefore the one whose guards matter most:
//!
//! * a turn the USER interrupted must never be re-driven (`is_natural_stop`);
//! * a natural completion must be re-driven;
//! * the live continue counter must reach the payload as `stop_hook_active` so
//!   a script can break its own loop.
//!
//! The fixture (`stop_journal.sh`) blocks unconditionally AND appends its raw
//! stdin payload to a journal file, so each assertion reads the real serialized
//! `status` / `stop_hook_active` the engine sent rather than inferring them.
//!
//! COUPLING (read before changing either side, and do NOT delete this test if
//! it starts failing — the failure is the point):
//!  1. `subagent::injection::inject_and_run_parent` emits its
//!     `parent_agent_stream` `"started"` event (step 3) BEFORE it builds and
//!     checks the model chain (step 4). That is what makes the re-drive
//!     observable in a test environment with NO provider configured. If
//!     "started" ever moves below the empty-chain bail-out, section 2 inverts
//!     from "arrives" to "never arrives".
//!  2. `hooks::stop_continue_inject` DISCARDS the `InjectionOutcome` and returns
//!     `true` on any resolvable session/agent, so the continue counter is NOT
//!     rolled back when the injection can't actually run a turn. That is what
//!     makes section 3 observe `stop_hook_active: true`. If it starts honoring
//!     the outcome, section 3 inverts to `false`.
//!
//! Deliberately NOT covered here: the full 1..3-honored / 4th-suppressed
//! `MAX_STOP_CONTINUES` sequence. A suppressed 4th fire has no positive signal
//! (nothing is emitted) and the ordering of the spawned fire-and-forget tasks is
//! only loosely observable, so asserting it here would be flaky theatre. That
//! math is already covered deterministically by
//! `hooks::guard_tests::stop_continue_counter_caps_resets_and_never_leaks` in
//! `crates/ha-core/src/hooks/mod.rs`.
//!
//! Single `#[tokio::test]` per binary: `install_hook` writes process-global
//! config and `reload_from_config()` swaps a global registry, so two test fns in
//! one binary would race on parallel threads.
//!
//! Unix-only: the hook shells out to `bash`.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use ha_core::event_bus::AppEvent;
use ha_core::hooks::{self, HooksConfig};
use tokio::sync::broadcast;

/// Absolute path to a fixture script, resolved at compile time against the
/// crate manifest dir so the test is location-independent.
fn fixture(name: &str) -> String {
    format!(
        "{}/tests/fixtures/hooks/claude-code-compat/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    )
}

/// Whether `jq` is callable. The official-shaped fixture depends on it; without
/// it the suite can't run authentically, so we skip instead of asserting.
fn jq_available() -> bool {
    Command::new("jq")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Install the journaling Stop hook and reload the registry so the next
/// `fire_stop` picks it up. Same shape as `hooks_compat.rs::install_hook`, plus
/// the journal path as `$1` (the handler runs `bash -c "<command>"`, so the
/// argument is passed through; single-quoted for paths with spaces).
fn install_stop_journal_hook(script: &str, log: &Path) {
    let cfg: HooksConfig = serde_json::from_str(&format!(
        r#"{{ "Stop": [ {{ "hooks": [ {{ "type": "command", "shell": "bash", "command": "bash {script} '{log}'" }} ] }} ] }}"#,
        log = log.display()
    ))
    .expect("parse hooks config");
    ha_core::config::mutate_config(("hooks", "test"), |c| {
        c.hooks = cfg.clone();
        Ok(())
    })
    .expect("write hooks config");
    hooks::registry::reload_from_config();
}

/// Parse the journal into one JSON value per hook invocation. A trailing
/// partially-written line is skipped rather than panicking, so this is safe to
/// call in a polling loop.
fn journal_records(log: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .collect()
}

/// Poll until the journal holds at least `at_least` records, or panic with the
/// raw journal contents. `fire_stop` is fire-and-forget (it spawns), so every
/// observation in this test is a wait-with-deadline.
async fn wait_for_journal(log: &Path, at_least: usize, what: &str) -> Vec<serde_json::Value> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let records = journal_records(log);
        if records.len() >= at_least {
            return records;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "stop_journal.sh must journal one payload per Stop dispatch — waited 30s for \
             {at_least} record(s) after {what}, saw {}. Raw journal: {:?}",
            records.len(),
            std::fs::read_to_string(log).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether `ev` is the `parent_agent_stream` `"started"` event that a Stop-hook
/// continue re-drive of `session_id` emits. Payload is camelCase
/// (`ParentAgentStreamEvent` is `rename_all = "camelCase"`) and the run id is
/// prefixed `stop-hook-continue-` by `hooks::stop_continue_inject`.
fn is_stop_continue_started(ev: &AppEvent, session_id: &str) -> bool {
    let field = |k: &str| ev.payload.get(k).and_then(|v| v.as_str());
    ev.name == "parent_agent_stream"
        && field("eventType") == Some("started")
        && field("parentSessionId") == Some(session_id)
        && field("runId").is_some_and(|r| r.starts_with("stop-hook-continue-"))
}

/// Wait up to `within` for a Stop-continue `"started"` event for `session_id`.
/// Unrelated bus traffic (`config:changed`, …) is skipped; a lagged receiver
/// keeps reading rather than aborting the wait.
async fn wait_stop_continue_started(
    rx: &mut broadcast::Receiver<AppEvent>,
    session_id: &str,
    within: Duration,
) -> Option<AppEvent> {
    tokio::time::timeout(within, async {
        loop {
            match rx.recv().await {
                Ok(ev) if is_stop_continue_started(&ev, session_id) => return Some(ev),
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_hook_block_to_continue_respects_the_natural_stop_gate() {
    if !jq_available() {
        eprintln!(
            "SKIP hooks_stop_continue: `jq` not installed — the official-shaped \
             Stop fixture needs it. (CI runners ship jq; this skip only affects \
             jq-less local environments.)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HA_DATA_DIR", tmp.path());
    ha_core::paths::ensure_dirs().expect("ensure_dirs");
    ha_core::init_runtime("test");

    // `inject_and_run_parent` bails before emitting anything unless a real
    // session row exists (`parent_session_present`), so the re-drive is only
    // observable for a session that is actually in the DB.
    let session = ha_core::get_session_db()
        .expect("session db initialized by init_runtime")
        .create_session("ha-main")
        .expect("create parent session row");
    let sid = session.id.clone();

    let log = tmp.path().join("stop-journal.jsonl");
    install_stop_journal_hook(&fixture("stop_journal.sh"), &log);

    // Subscribe BEFORE any fire — `fire_stop` spawns, and a subscription taken
    // afterwards can miss the event entirely.
    let bus = ha_core::globals::EVENT_BUS
        .get()
        .expect("event bus initialized by init_runtime")
        .clone();
    let mut rx = bus.subscribe();

    // 1. INTERRUPTED must not be re-driven. `stop_journal.sh` blocks on every
    //    invocation, so the only thing that can suppress the continue is
    //    `fire_stop`'s `is_natural_stop` gate — a Stop hook must never resurrect
    //    a turn the user interrupted.
    hooks::fire_stop(&sid, Some("ha-main"), "interrupted", Some("partial"));
    // Positively confirm the hook RAN first: without this the negative assertion
    // below would pass just as happily against a mis-wired hook that never fired.
    let records = wait_for_journal(&log, 1, "fire_stop(status=interrupted)").await;
    assert_eq!(
        records[0].get("status").and_then(|v| v.as_str()),
        Some("interrupted"),
        "stop_journal.sh must receive the terminal turn status as `.status` — \
         got payload {:?}",
        records[0]
    );
    // Now the real claim: no re-drive. Generous window; the continue decision in
    // `fire_stop` happens immediately after the dispatch we just observed.
    let leaked = wait_stop_continue_started(&mut rx, &sid, Duration::from_secs(5)).await;
    assert!(
        leaked.is_none(),
        "an interrupted turn must NOT be re-driven even though stop_journal.sh blocked \
         (fire_stop's is_natural_stop gate), but a stop-hook-continue re-drive started: {leaked:?}"
    );

    // 2. COMPLETED is re-driven. Same blocking fixture, only `status` differs —
    //    so an arriving `started` event isolates the gate as the cause.
    hooks::fire_stop(&sid, Some("ha-main"), "completed", Some("all done"));
    let started = wait_stop_continue_started(&mut rx, &sid, Duration::from_secs(30)).await;
    let started = started.expect(
        "a completed turn whose Stop hook blocked (exit 2) must be re-driven — no \
         parent_agent_stream \"started\" event with a `stop-hook-continue-` runId arrived \
         within 30s (see the COUPLING note: \"started\" is emitted BEFORE the model-chain \
         check, so this holds with no provider configured)",
    );
    let records = wait_for_journal(&log, 2, "fire_stop(status=completed)").await;
    assert_eq!(
        records[1].get("status").and_then(|v| v.as_str()),
        Some("completed"),
        "the second journal record must be the completed fire — got payload {:?}",
        records[1]
    );
    assert_eq!(
        records[1].get("stop_hook_active").and_then(|v| v.as_bool()),
        Some(false),
        "the FIRST Stop of a fresh turn must report `stop_hook_active: false` (no continue \
         loop is running yet) — got payload {:?}, re-drive event {started:?}",
        records[1]
    );

    // 3. Re-entrancy. Section 2 is fully settled (its re-drive already started),
    //    so this fire is guaranteed to see the bumped live counter. The official
    //    contract is that `stop_hook_active` lets a script detect "the turn ended
    //    again because I asked for it" and stop blocking; if the counter never
    //    reached the payload, every official Stop hook would loop until the
    //    engine's own cap cut it off.
    hooks::fire_stop(&sid, Some("ha-main"), "completed", Some("second pass"));
    let records = wait_for_journal(&log, 3, "the second fire_stop(status=completed)").await;
    assert_eq!(
        records[2].get("stop_hook_active").and_then(|v| v.as_bool()),
        Some(true),
        "a Stop firing while a Stop-hook continue loop is active must report \
         `stop_hook_active: true` so the script can break its own loop — got payload {:?}",
        records[2]
    );
}
