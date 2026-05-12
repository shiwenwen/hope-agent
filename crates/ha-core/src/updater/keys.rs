//! Single source of truth for the Minisign verification key.
//!
//! `src-tauri/tauri.conf.json#updater.pubkey` carries the same value in
//! base64-of-the-full-pubkey-file form so `tauri-plugin-updater` keeps
//! working in the desktop path. The startup sanity check
//! [`assert_pubkey_matches_tauri_conf`] (wired from `init_runtime`) refuses
//! to boot if the two ever drift — without that gate, a desktop release and
//! a `hope-agent server` self-update could verify against different keys.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use minisign_verify::PublicKey;

/// The full Minisign pubkey file (two lines: `untrusted comment:` + base64
/// body), base64-encoded so it can be embedded as a single literal — same
/// shape `tauri-plugin-updater` consumes from `tauri.conf.json`.
///
/// Sync the literal in `src-tauri/tauri.conf.json#updater.pubkey` whenever
/// this changes. `scripts/verify-updater-pubkey.mjs` (run in CI) diffs the
/// two so a drift can't ship.
pub const MINISIGN_PUBKEY_BASE64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMxNjgyN0E5ODY0MzM4RDIKUldUU09FT0dxU2RvTVRGWW5IN3lkMkE2b05mRW4wUk0wak5yUHQvMjNNaTdXYVR6RjlxbmVUcC8K";

/// Parse [`MINISIGN_PUBKEY_BASE64`] into the verifier used by every
/// signature check in this crate. Each call re-parses (cheap, ~64 bytes)
/// so the verifier stays `Send` without needing a global cache.
pub fn pubkey() -> Result<PublicKey> {
    let raw = B64
        .decode(MINISIGN_PUBKEY_BASE64.trim())
        .context("MINISIGN_PUBKEY_BASE64 is not valid base64")?;
    let text =
        std::str::from_utf8(&raw).context("MINISIGN_PUBKEY_BASE64 does not decode to UTF-8")?;
    PublicKey::decode(text).map_err(|e| anyhow::anyhow!("invalid minisign pubkey: {e}"))
}

/// Boot-time guard: refuse to start if `tauri.conf.json#updater.pubkey`
/// and [`MINISIGN_PUBKEY_BASE64`] have drifted. Called by `init_runtime`;
/// also exposed as a public function so the CI verifier script can replay
/// the same comparison without spawning a full binary.
pub fn assert_pubkey_matches_tauri_conf(tauri_conf_pubkey: &str) -> Result<()> {
    if tauri_conf_pubkey.trim() != MINISIGN_PUBKEY_BASE64.trim() {
        bail!(
            "Minisign pubkey drift between tauri.conf.json#updater.pubkey \
             and ha-core::updater::keys::MINISIGN_PUBKEY_BASE64 — desktop \
             updater and self-update would verify against different keys. \
             Re-sync both literals (or regenerate the key, then update them \
             together) before booting."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pubkey_parses_from_embedded_constant() {
        let pk = pubkey().expect("embedded pubkey must parse");
        // The Tauri-shipped pubkey decodes to a 32-byte Ed25519 key plus
        // 8-byte key id and 2-byte signature alg — `minisign-verify`
        // doesn't expose those internals, so the smoke test is just
        // "does it parse". Drift in the literal will fail here.
        let _ = pk; // keep `pk` live so the parse is the actual assertion.
    }

    #[test]
    fn sanity_check_accepts_identical_value() {
        assert_pubkey_matches_tauri_conf(MINISIGN_PUBKEY_BASE64).unwrap();
    }

    #[test]
    fn sanity_check_rejects_drift() {
        let err = assert_pubkey_matches_tauri_conf("different").unwrap_err();
        assert!(err.to_string().contains("drift"));
    }

    #[test]
    fn sanity_check_tolerates_whitespace_around_value() {
        // `tauri.conf.json` writers sometimes wrap long literals; the
        // round-trip from disk should still compare equal.
        let padded = format!("\n {}\n", MINISIGN_PUBKEY_BASE64);
        assert_pubkey_matches_tauri_conf(&padded).unwrap();
    }
}
