//! Update manifest fetch + parse + platform selection.
//!
//! The manifest is the same `latest.json` `tauri-plugin-updater` consumes
//! (so the desktop path doesn't change), extended with a `bare_binary`
//! section that the headless self-contained updater consumes. The
//! `bare_binary` field is optional — if release.yml hasn't published the
//! tar.gz/zip for a given version yet, headless callers fall back to the
//! package-manager path (or surface the gap to the user via
//! `ask_user_question`).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// Endpoint matching `src-tauri/tauri.conf.json#updater.endpoints[0]` so the
/// desktop + headless paths read the same manifest.
pub const UPDATE_MANIFEST_URL: &str =
    "https://github.com/shiwenwen/hope-agent/releases/latest/download/latest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// `tauri-action` writes this without the `v` prefix (e.g. `"0.2.1"`).
    pub version: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub pub_date: Option<String>,
    /// Per-platform installer URL + Minisign signature. Same shape Tauri ships.
    pub platforms: BTreeMap<String, PlatformEntry>,
    /// Self-update extension: per-platform bare-binary archive URL + sig.
    /// Absent on releases that pre-date the extension; callers must tolerate
    /// `None` and route to the package-manager path or prompt the user.
    #[serde(default)]
    pub bare_binary: BareBinaryRoot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformEntry {
    pub url: String,
    pub signature: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BareBinaryRoot {
    #[serde(default)]
    pub platforms: BTreeMap<String, BareBinaryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BareBinaryEntry {
    pub url: String,
    pub signature: String,
    /// `tar.gz` (Unix) or `zip` (Windows). Used by `download` to pick the
    /// extractor without sniffing magic bytes.
    pub archive: ArchiveKind,
    /// Path to the executable inside the archive. Always uses `/` separators
    /// regardless of the host OS — extractors normalize.
    pub binary_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

/// Default reqwest timeout for the manifest fetch — short, since it's a
/// single small JSON over CDN-fronted GitHub Releases. Set conservatively
/// so a stalled CI / metered network doesn't hang the chat for minutes.
const MANIFEST_FETCH_TIMEOUT_SECS: u64 = 20;

pub async fn fetch_manifest() -> Result<Manifest> {
    fetch_manifest_from(UPDATE_MANIFEST_URL).await
}

pub async fn fetch_manifest_from(url: &str) -> Result<Manifest> {
    // SSRF gate on every outbound HTTP for the self-update path. The
    // manifest URL is read from a const today, but a future config knob
    // for staging/beta could expose it — fail closed before we even open
    // the connection. `Default` policy blocks private / link-local /
    // metadata IPs (the real SSRF concerns) but still allows loopback so
    // local mirrors / test wiremock servers work without ceremony.
    let ssrf_cfg = &crate::config::cached_config().ssrf;
    crate::security::ssrf::check_url(
        url,
        crate::security::ssrf::SsrfPolicy::Default,
        &ssrf_cfg.trusted_hosts,
    )
    .await
    .with_context(|| format!("SSRF check failed for manifest URL {url}"))?;
    let builder =
        reqwest::Client::builder().timeout(Duration::from_secs(MANIFEST_FETCH_TIMEOUT_SECS));
    let client = crate::provider::apply_proxy_for_url(builder, url)
        .build()
        .context("reqwest client build failed")?;
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("fetch {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("manifest fetch returned HTTP {status} from {url}");
    }
    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("read manifest body from {url}"))?;
    serde_json::from_slice(&bytes).with_context(|| {
        let preview = String::from_utf8_lossy(&bytes);
        let head = crate::truncate_utf8(&preview, 256);
        format!("parse manifest JSON ({head})")
    })
}

/// Stable key under which Tauri / our release.yml records platform entries.
/// Format matches what `tauri-action` writes: `<os>-<arch>` where `os ∈
/// {darwin, linux, windows}` and `arch ∈ {x86_64, aarch64}`.
pub fn current_platform_key() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "darwin-x86_64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "darwin-aarch64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux-aarch64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x86_64"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "windows-aarch64"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
    )))]
    {
        "unknown"
    }
}

pub fn select_platform_entry<'a>(manifest: &'a Manifest, key: &str) -> Option<&'a PlatformEntry> {
    manifest.platforms.get(key)
}

pub fn select_bare_binary<'a>(manifest: &'a Manifest, key: &str) -> Option<&'a BareBinaryEntry> {
    manifest.bare_binary.platforms.get(key)
}

/// `true` iff `latest > current` per a numeric `X.Y.Z` compare. Both inputs
/// tolerate a leading `v` and an optional `-prerelease` suffix (the suffix
/// is treated as "older than the same numeric version without it" so we
/// don't accidentally push a `-rc1` over a stable). Returns `false` for
/// anything unparseable so a broken manifest never strands users on the
/// "update available" banner forever.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

fn parse_version(s: &str) -> Option<(u32, u32, u32, u8)> {
    let trimmed = s.trim().trim_start_matches('v');
    let (numeric, suffix) = match trimmed.split_once('-') {
        Some((n, _pre)) => (n, 0u8), // any prerelease ranks below stable
        None => (trimmed, 1u8),
    };
    let mut parts = numeric.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch, suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_platform_key_is_one_of_the_known_keys() {
        let k = current_platform_key();
        assert!(
            matches!(
                k,
                "darwin-x86_64"
                    | "darwin-aarch64"
                    | "linux-x86_64"
                    | "linux-aarch64"
                    | "windows-x86_64"
                    | "windows-aarch64"
                    | "unknown"
            ),
            "unexpected key: {k}"
        );
    }

    #[test]
    fn parses_minimal_tauri_manifest() {
        let body = r#"{
            "version": "0.2.1",
            "notes": "fix self-update bug",
            "pub_date": "2026-05-12T10:00:00Z",
            "platforms": {
                "darwin-aarch64": {
                    "url": "https://example/Hope.dmg",
                    "signature": "RUR..."
                }
            }
        }"#;
        let m: Manifest = serde_json::from_str(body).unwrap();
        assert_eq!(m.version, "0.2.1");
        assert_eq!(m.platforms.len(), 1);
        assert!(m.bare_binary.platforms.is_empty());
        assert!(select_platform_entry(&m, "darwin-aarch64").is_some());
        assert!(select_bare_binary(&m, "darwin-aarch64").is_none());
    }

    #[test]
    fn parses_manifest_with_bare_binary_extension() {
        let body = r#"{
            "version": "0.2.1",
            "platforms": {},
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
        let m: Manifest = serde_json::from_str(body).unwrap();
        let entry = select_bare_binary(&m, "linux-x86_64").unwrap();
        assert_eq!(entry.archive, ArchiveKind::TarGz);
        assert_eq!(entry.binary_path, "hope-agent");
    }

    #[test]
    fn is_newer_compares_semver_numerically() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v0.2.10", "0.2.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
    }

    #[test]
    fn is_newer_treats_prerelease_as_older_than_stable() {
        // 0.2.0 (stable) > 0.2.0-rc1 — never push a prerelease over a stable.
        assert!(!is_newer("0.2.0-rc1", "0.2.0"));
        assert!(is_newer("0.2.0", "0.2.0-rc1"));
    }

    #[test]
    fn is_newer_returns_false_on_unparseable_input() {
        assert!(!is_newer("not-a-version", "0.1.0"));
        assert!(!is_newer("0.1.0", "garbage"));
        assert!(!is_newer("", ""));
    }
}
