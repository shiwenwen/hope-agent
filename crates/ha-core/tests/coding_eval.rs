use ha_core::coding_eval;

#[tokio::test(flavor = "current_thread")]
async fn coding_control_plane_fixtures_pass() {
    let temp = tempfile::tempdir().expect("create tempdir");
    std::env::set_var("HA_DATA_DIR", temp.path());
    ha_core::init_runtime("test");
    let db = ha_core::get_session_db()
        .cloned()
        .expect("session db initialized");

    let fixtures = coding_eval::load_fixtures().expect("load coding eval fixtures");
    assert!(
        fixtures.len() >= 3,
        "expected at least three coding eval fixtures"
    );

    let mut total_checks = 0usize;
    let mut failures = Vec::new();
    let mut metric_lines = Vec::new();

    for fixture in &fixtures {
        let report = coding_eval::evaluate(db.clone(), fixture)
            .await
            .unwrap_or_else(|err| panic!("fixture {} failed to run: {err:#}", fixture.name));
        assert!(
            !report.outcomes.is_empty(),
            "fixture {} produced no checks",
            report.name
        );
        total_checks += report.outcomes.len();
        metric_lines.push(format!(
            "{} precision={:?} recall={:?} review_findings={:?} commands={:?}",
            report.name,
            report.metrics.context_precision,
            report.metrics.critical_context_recall,
            report.metrics.review_findings,
            report.metrics.verification_commands
        ));
        for failure in report.failures() {
            failures.push(format!(
                "[{}] {}: {}",
                report.name, failure.name, failure.detail
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} coding eval check(s) failed:\n{}\n\nmetrics:\n{}",
        failures.len(),
        failures.join("\n"),
        metric_lines.join("\n")
    );
    eprintln!(
        "coding eval: {} fixtures, {} checks passed\n{}",
        fixtures.len(),
        total_checks,
        metric_lines.join("\n")
    );
}
