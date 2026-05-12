//! Minisign signature verification for downloaded update artifacts.
//!
//! Reuses the Ed25519 pubkey that `tauri-plugin-updater` already verifies
//! desktop releases against ([`crate::updater::keys`]) so we don't need a
//! second key in the secret store. The signature blob format is the same
//! one tauri-action writes — `signature: <base64-minisign-blob>` inside
//! `latest.json` for both the desktop installers and the bare-binary
//! archive entries we add via `scripts/patch-latest-json.mjs`.

use anyhow::Result;
use minisign_verify::Signature;

use super::keys;

/// Verify `payload` against an inline `signature` string (the same blob
/// tauri-action writes into `latest.json#platforms.*.signature` and
/// `patch-latest-json.mjs` writes into `bare_binary.platforms.*.signature`).
pub fn verify_bytes(payload: &[u8], signature: &str) -> Result<()> {
    let pk = keys::pubkey()?;
    let sig = Signature::decode(signature.trim())
        .map_err(|e| anyhow::anyhow!("decode minisign signature: {e}"))?;
    pk.verify(payload, &sig, /*allow_legacy=*/ false)
        .map_err(|e| anyhow::anyhow!("minisign verify failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity: an empty / malformed signature blob fails fast with a
    /// recognisable error (not a panic).
    #[test]
    fn empty_signature_returns_decode_error() {
        let err = verify_bytes(b"hello", "").unwrap_err();
        assert!(err.to_string().contains("decode minisign signature"));
    }

    #[test]
    fn garbage_signature_returns_decode_error() {
        let err = verify_bytes(b"hello", "not-a-real-minisign-blob").unwrap_err();
        assert!(err.to_string().contains("decode minisign signature"));
    }

    // The corresponding "valid signature → Ok" test lives in
    // crates/ha-core/tests/updater_e2e.rs where we can generate a fresh
    // keypair, sign a payload, and verify against that pubkey. We can't do
    // that here without unsafely swapping out the embedded production
    // pubkey, which would defeat the point of the constant.
}
