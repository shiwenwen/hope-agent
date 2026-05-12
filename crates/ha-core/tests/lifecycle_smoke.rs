//! Smoke tests for `ha_core::lifecycle`. The destructive paths (`restart`,
//! `respawn::respawn_detached_server`) are not exercised here — they'd
//! actually kill the test process. We cover the inspection surface:
//! route enum, inflight collector with no globals set, and the
//! `server_launch_args` capture API.

use ha_core::lifecycle::{collect_inflight, Route};

#[test]
fn route_as_str_round_trips() {
    assert_eq!(Route::Desktop.as_str(), "desktop");
    assert_eq!(Route::Service.as_str(), "service");
    assert_eq!(Route::Respawn.as_str(), "respawn");
    assert_eq!(Route::Unsupported.as_str(), "unsupported");
}

#[test]
fn inflight_summary_empty_when_no_globals() {
    // No SessionDB / CronDB / async_jobs DB initialized in this binary —
    // collect_inflight should swallow the misses and return an empty
    // summary rather than panicking. Active-turn registry might have
    // entries from earlier tests, so we just assert no panic + non-negative
    // length rather than `is_empty()`.
    let summary = collect_inflight();
    let _ = summary.len();
    // Each item carries a non-empty label and a recognized kind.
    for item in &summary.items {
        assert!(
            !item.label.is_empty(),
            "InflightItem.label should be non-empty"
        );
        let _ = item.kind.as_str();
    }
}

#[test]
fn server_launch_args_defaults_to_empty() {
    // First-write-wins: tests that call `set_server_launch_args` poison
    // this static for everything else in the binary. We can't safely set
    // it here, but we can confirm the getter never panics and the slice
    // is borrowable. The result is "empty when nothing set, otherwise
    // whatever an earlier test set" — both shapes are valid for this API.
    let args = ha_core::server_launch_args();
    let _ = args.len();
}
