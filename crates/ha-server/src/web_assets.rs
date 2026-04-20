//! Web GUI static-file serving for the embedded HTTP server.
//!
//! Serves the Vite-built front-end (`dist/`) as the axum router
//! `fallback_service` so users can point any browser at the server and
//! get the full React UI. Authentication still happens at the `/api` and
//! `/ws` layers via the existing middleware; static assets are open so
//! the login-like first paint works without a cookie / header round-trip.
//!
//! Resolution order (see [`resolve_strategy`]):
//!
//! 1. `HA_WEB_ROOT` env var pointing at a directory with `index.html` —
//!    wins for development overrides.
//! 2. `rust-embed` bundle baked into the binary — the release default.
//! 3. `Unavailable` — the front-end was never built. The fallback still
//!    renders a small placeholder HTML page pointing the user at the
//!    `npm run build` command, so the API continues to work while the
//!    Web GUI self-diagnoses.

use axum::{
    body::Body,
    http::{header, HeaderValue, Response, StatusCode, Uri},
};
use rust_embed::RustEmbed;
use std::path::PathBuf;

/// The Vite build output. In debug builds `rust-embed` reads files from
/// disk on each request (see `debug-embed` feature in the crate), so the
/// front-end can be iterated on without rebuilding `ha-server`. In
/// release builds the files are compiled into the binary.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../dist"]
#[prefix = ""]
struct FrontendAssets;

#[derive(Debug)]
pub enum WebAssetStrategy {
    /// Files read from a directory on disk (dev override).
    ServeDir(PathBuf),
    /// Files baked into the binary via `rust-embed`.
    Embedded,
    /// No `dist/` directory / bundle found — fall back to the diagnostic page.
    Unavailable,
}

pub fn resolve_strategy() -> WebAssetStrategy {
    if let Ok(path) = std::env::var("HA_WEB_ROOT") {
        let candidate = PathBuf::from(&path);
        if candidate.join("index.html").exists() {
            return WebAssetStrategy::ServeDir(candidate);
        }
        eprintln!(
            "[ha-server] HA_WEB_ROOT={} does not contain index.html — falling back to embedded assets",
            path
        );
    }

    if FrontendAssets::get("index.html").is_some() {
        WebAssetStrategy::Embedded
    } else {
        WebAssetStrategy::Unavailable
    }
}

/// axum handler for the embedded-assets branch. Unknown non-API paths
/// fall back to `index.html` so client-side React Router routes work.
pub async fn serve_embedded(uri: Uri) -> Response<Body> {
    let raw = uri.path().trim_start_matches('/');
    let asset_path = if raw.is_empty() { "index.html" } else { raw };

    if let Some(file) = FrontendAssets::get(asset_path) {
        return build_response(asset_path, file.data.into_owned(), StatusCode::OK);
    }

    // SPA fallback — serve index.html for any unknown path. React Router
    // takes over on the client.
    if let Some(index) = FrontendAssets::get("index.html") {
        return build_response("index.html", index.data.into_owned(), StatusCode::OK);
    }

    serve_unavailable_notice().await
}

/// Fallback used when neither `HA_WEB_ROOT` nor the embedded bundle
/// contain assets. Returns a static HTML page instead of a bare 404 so
/// the user immediately sees what's wrong.
pub async fn serve_unavailable_notice() -> Response<Body> {
    let body = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Hope Agent — Web GUI unavailable</title>
<style>body{font-family:system-ui,sans-serif;background:#0b0d11;color:#e6e6e6;
display:flex;min-height:100vh;align-items:center;justify-content:center;margin:0}
main{max-width:560px;padding:2rem;border:1px solid #2a2d33;border-radius:12px;background:#14171c}
code{background:#1f232a;padding:.15rem .35rem;border-radius:4px}</style></head>
<body><main><h1>Web GUI not available</h1>
<p>The front-end was not bundled with this build. Run <code>npm run build</code>
in the project root and restart <code>hope-agent server</code>, or set the
<code>HA_WEB_ROOT</code> environment variable to a directory containing the
Vite <code>dist/</code> output.</p>
<p>API endpoints remain available under <code>/api</code>.</p>
</main></body></html>"#;

    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body))
        .expect("static response")
}

fn build_response(path: &str, bytes: Vec<u8>, status: StatusCode) -> Response<Body> {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mime = HeaderValue::from_str(mime.as_ref())
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .body(Body::from(bytes))
        .expect("valid static response")
}
