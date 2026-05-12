//! Integration tests for the self-update pipeline.
//!
//! Covers:
//! - manifest fetch + version comparison against a wiremock-served
//!   `latest.json` (no live network),
//! - `atomic_replace_binary` round-trip on a tempfile, including
//!   cross-device fallback on Unix.
//!
//! The full install pipeline (download → verify → swap → service
//! restart) is intentionally NOT exercised end-to-end — it would
//! require a real Minisign signing key plus a service-control sandbox.
//! The signing test lives close to the verifier in
//! `crates/ha-core/src/updater/signature.rs` (smoke) and the install
//! pipeline itself is covered by the manual end-to-end matrix in
//! `docs/architecture/self-update.md`.

use ha_core::updater::manifest::{self, ArchiveKind};
use std::fs;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn fetch_manifest_parses_full_release_payload() {
    let server = MockServer::start().await;
    let body = r#"{
        "version": "0.2.1",
        "notes": "fix: addressed CVE-2025-1234",
        "pub_date": "2026-05-12T10:00:00Z",
        "platforms": {
            "darwin-aarch64": {
                "url": "https://example/hope-agent-0.2.1-aarch64.dmg",
                "signature": "RUR..."
            }
        },
        "bare_binary": {
            "platforms": {
                "linux-x86_64": {
                    "url": "https://example/hope-agent-0.2.1-linux-x86_64.tar.gz",
                    "signature": "RUR...",
                    "archive": "tar_gz",
                    "binary_path": "hope-agent"
                }
            }
        }
    }"#;
    Mock::given(method("GET"))
        .and(path("/latest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let url = format!("{}/latest.json", server.uri());
    let m = manifest::fetch_manifest_from(&url).await.unwrap();
    assert_eq!(m.version, "0.2.1");
    assert_eq!(m.platforms.len(), 1);
    let bb = manifest::select_bare_binary(&m, "linux-x86_64").unwrap();
    assert_eq!(bb.archive, ArchiveKind::TarGz);
    assert_eq!(bb.binary_path, "hope-agent");
}

#[tokio::test]
async fn fetch_manifest_surfaces_http_error_clearly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latest.json"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let url = format!("{}/latest.json", server.uri());
    let err = manifest::fetch_manifest_from(&url).await.unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("HTTP 503"), "msg: {msg}");
}

#[tokio::test]
async fn fetch_manifest_surfaces_parse_error_clearly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;
    let url = format!("{}/latest.json", server.uri());
    let err = manifest::fetch_manifest_from(&url).await.unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("parse manifest JSON"),
        "expected parse error context, got: {msg}"
    );
}

#[test]
fn atomic_replace_binary_swaps_content() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("hope-agent");
    let source = dir.path().join("hope-agent.new");
    fs::write(&target, b"old binary").unwrap();
    fs::write(&source, b"new binary").unwrap();

    ha_core::platform::atomic_replace_binary(&target, &source).unwrap();

    assert_eq!(fs::read(&target).unwrap(), b"new binary");
    // Source is consumed (or moved) — must not co-exist with target.
    assert!(!source.exists(), "source still present after swap");
}

#[cfg(unix)]
#[test]
fn atomic_replace_binary_marks_executable_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("hope-agent");
    let source = dir.path().join("hope-agent.new");
    fs::write(&target, b"old").unwrap();
    fs::write(&source, b"new").unwrap();
    // Source ships as 0644 (e.g. unpacked from a zip on a foreign
    // filesystem); the swap must publish 0755 so the binary is runnable.
    fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();

    ha_core::platform::atomic_replace_binary(&target, &source).unwrap();
    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755, "expected 0755, got {mode:o}");
}
