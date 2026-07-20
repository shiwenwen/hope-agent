//! Web GUI static-file serving for the embedded HTTP server.
//!
//! Serves the Vite-built front-end (`dist/`) as the axum router
//! `fallback_service` so users can point any browser at the server and
//! get the full React UI. Authentication still happens at the `/api` and
//! `/ws` layers via the existing middleware; static assets are open so
//! the login-like first paint works without a cookie / header round-trip.
//!
//! The bundle is produced by `build.rs`: every asset is brotli-compressed at
//! build time and stored compressed in the binary, then handed to the browser
//! as-is with `Content-Encoding: br`. Clients that do not advertise brotli get
//! the bytes decompressed on the way out — correct, just slower, and in
//! practice unreachable since every engine that can run this UI supports br.
//!
//! Resolution order (see [`resolve_strategy`]):
//!
//! 1. `HA_WEB_ROOT` env var pointing at a directory with `index.html` —
//!    wins for development overrides.
//! 2. The bundle baked in by `build.rs` — the release default.
//! 3. The `dist/` directory on disk — how debug builds serve, since they
//!    deliberately skip the (slow) build-time compression.
//! 4. `Unavailable` — the front-end was never built. The fallback still
//!    renders a small placeholder HTML page pointing the user at the
//!    `pnpm build` command, so the API continues to work while the
//!    Web GUI self-diagnoses.

use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderValue, Response, StatusCode, Uri},
};
use std::path::PathBuf;

mod bundle {
    include!(concat!(env!("OUT_DIR"), "/frontend_assets.rs"));
}

#[derive(Debug)]
pub enum WebAssetStrategy {
    /// Files read from a directory on disk (dev override, or any debug build).
    ServeDir(PathBuf),
    /// Files baked into the binary by `build.rs`.
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

    if lookup("index.html").is_some() {
        return WebAssetStrategy::Embedded;
    }

    // Debug builds carry no bundle; serve the Vite output straight from disk.
    let dist = PathBuf::from(bundle::DIST_DIR);
    if dist.join("index.html").exists() {
        return WebAssetStrategy::ServeDir(dist);
    }

    WebAssetStrategy::Unavailable
}

/// One asset as stored in the binary.
struct Asset {
    bytes: &'static [u8],
    /// `true` when `bytes` is a brotli stream rather than the raw file.
    compressed: bool,
}

fn lookup(path: &str) -> Option<Asset> {
    let idx = bundle::ASSET_INDEX
        .binary_search_by(|(p, ..)| (*p).cmp(path))
        .ok()?;
    let (_, offset, stored, raw) = bundle::ASSET_INDEX[idx];
    let start = offset as usize;
    let bytes = &bundle::ASSET_BLOB[start..start + stored as usize];
    Some(Asset {
        bytes,
        // build.rs only stores raw when compression did not pay, and always
        // shrinks by at least 5% otherwise — so equal lengths mean raw.
        compressed: stored != raw,
    })
}

/// axum handler for the embedded-assets branch. Unknown non-API paths
/// fall back to `index.html` so client-side React Router routes work.
pub async fn serve_embedded(headers: HeaderMap, uri: Uri) -> Response<Body> {
    let raw = uri.path().trim_start_matches('/');
    let asset_path = if raw.is_empty() { "index.html" } else { raw };

    if let Some(asset) = lookup(asset_path) {
        return build_response(asset_path, asset, &headers);
    }

    // SPA fallback — serve index.html for any unknown path. React Router
    // takes over on the client.
    if let Some(index) = lookup("index.html") {
        return build_response("index.html", index, &headers);
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
<p>The front-end was not bundled with this build. Run <code>pnpm build</code>
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

fn accepts_brotli(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                // Strip any q-value before matching so `br;q=0.9` still counts.
                .any(|part| part.split(';').next().is_some_and(|e| e.trim() == "br"))
        })
}

fn build_response(path: &str, asset: Asset, headers: &HeaderMap) -> Response<Body> {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mime = HeaderValue::from_str(mime.as_ref())
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));

    let builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime);

    if !asset.compressed {
        return builder
            .body(Body::from(asset.bytes))
            .expect("valid static response");
    }

    if accepts_brotli(headers) {
        return builder
            .header(header::CONTENT_ENCODING, HeaderValue::from_static("br"))
            // Cached responses must not be reused for a client that cannot
            // decode brotli.
            .header(header::VARY, HeaderValue::from_static("accept-encoding"))
            .body(Body::from(asset.bytes))
            .expect("valid static response");
    }

    let mut out = Vec::new();
    match brotli::BrotliDecompress(&mut &asset.bytes[..], &mut out) {
        Ok(()) => builder
            .header(header::VARY, HeaderValue::from_static("accept-encoding"))
            .body(Body::from(out))
            .expect("valid static response"),
        Err(e) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(format!("asset decode failed: {e}")))
            .expect("static response"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    #[test]
    fn detects_brotli_support() {
        assert!(accepts_brotli(&hdr("br")));
        assert!(accepts_brotli(&hdr("gzip, deflate, br")));
        assert!(accepts_brotli(&hdr("gzip, deflate, br;q=0.9")));
        assert!(accepts_brotli(&hdr("br;q=1.0, gzip;q=0.8")));
    }

    #[test]
    fn rejects_when_brotli_absent() {
        assert!(!accepts_brotli(&HeaderMap::new()));
        assert!(!accepts_brotli(&hdr("gzip, deflate")));
        // `brotli` is not `br` — must not match on a prefix.
        assert!(!accepts_brotli(&hdr("gzip, brotli")));
    }

    #[test]
    fn index_is_bundled_or_on_disk() {
        // Whichever build profile this runs under, the strategy must resolve to
        // something servable rather than the diagnostic page.
        assert!(
            !matches!(resolve_strategy(), WebAssetStrategy::Unavailable),
            "front-end assets unavailable in both bundle and dist/"
        );
    }
}
