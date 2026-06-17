//! Deterministic golden-fixture eval for next-gen Dreaming (design §9, PR #10).
//!
//! Drives [`ha_core::memory::dreaming::eval`] over the JSON fixtures in
//! `tests/fixtures/dreaming/`. This is the §9.3 **deterministic layer** — no
//! LLM, so it runs in the default CI suite and locks in the safety red-lines
//! (scope isolation, stale suppression, evidence coverage, conflict→review,
//! legacy-sync hidden-set, evidence fail-closed).
//!
//! It runs as an integration test (own process) so it can own the claim-store
//! global exclusively; each fixture confines its seeds to a unique scope, so a
//! single shared DB has no cross-fixture interference.

use std::sync::Arc;

use ha_core::memory::dreaming::eval;
use ha_core::memory::{claims, SqliteMemoryBackend};

#[test]
fn golden_fixtures_pass() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let backend =
        Arc::new(SqliteMemoryBackend::open(&tmp.path().join("memory.db")).expect("open backend"));
    claims::init_claim_store(backend.clone());

    let fixtures = eval::load_fixtures().expect("load fixtures");
    assert!(
        fixtures.len() >= 6,
        "expected at least 6 golden fixtures (design §11), found {}",
        fixtures.len()
    );

    let mut failures: Vec<String> = Vec::new();
    let mut total_checks = 0usize;
    for fx in &fixtures {
        let report = eval::evaluate(backend.as_ref(), fx)
            .unwrap_or_else(|e| panic!("evaluate fixture '{}' errored: {e}", fx.name));
        total_checks += report.outcomes.len();
        assert!(
            !report.outcomes.is_empty(),
            "fixture '{}' declared no checks",
            fx.name
        );
        for f in report.failures() {
            failures.push(format!("[{}] {}: {}", report.name, f.name, f.detail));
        }
    }

    assert!(
        failures.is_empty(),
        "{} golden-fixture check(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    println!(
        "dreaming eval: {} fixtures, {} checks passed",
        fixtures.len(),
        total_checks
    );
}
