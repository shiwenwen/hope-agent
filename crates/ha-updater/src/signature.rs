//! Minisign signature verification for downloaded update artifacts.
//!
//! Reuses the Ed25519 pubkey that `tauri-plugin-updater` already verifies
//! desktop releases against ([`crate::keys`]) so we don't need a
//! second key in the secret store. The signature blob format is the same
//! one tauri-action writes — `signature: <base64-minisign-blob>` inside
//! `latest.json` for both the desktop installers and the bare-binary
//! archive entries we add via `scripts/patch-latest-json.mjs`.

use anyhow::Result;
use minisign_verify::{PublicKey, Signature};

use super::keys;

/// Verify `payload` against an inline `signature` string (the same blob
/// tauri-action writes into `latest.json#platforms.*.signature` and
/// `patch-latest-json.mjs` writes into `bare_binary.platforms.*.signature`).
pub fn verify_bytes(payload: &[u8], signature: &str) -> Result<()> {
    let pk = keys::pubkey()?;
    verify_with_key(&pk, payload, signature)
}

fn verify_with_key(pk: &PublicKey, payload: &[u8], signature: &str) -> Result<()> {
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

    // Public test vectors from rust-minisign-verify's MIT-licensed tests:
    // https://github.com/jedisct1/rust-minisign-verify/blob/0.3.0/src/lib.rs
    // This public key is unrelated to the release key; no signing key is used.
    const TEST_KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    const PREHASHED: &str = "untrusted comment: signature from minisign secret key
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=
trusted comment: timestamp:1556193335\tfile:test
y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==";
    const LEGACY: &str = "untrusted comment: signature from minisign secret key
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=
trusted comment: timestamp:1555779966\tfile:test
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";

    fn mutate_line(line: usize, mutate: impl FnOnce(&mut [u8])) -> String {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let mut lines: Vec<_> = PREHASHED.lines().map(str::to_owned).collect();
        let mut bytes = STANDARD.decode(&lines[line]).unwrap();
        mutate(&mut bytes);
        lines[line] = STANDARD.encode(bytes);
        lines.join("\n")
    }

    #[test]
    fn prehashed_signature_accepts_only_matching_payload_and_key() {
        let key = PublicKey::from_base64(TEST_KEY).unwrap();
        assert!(verify_with_key(&key, b"test", PREHASHED).is_ok());
        assert!(verify_with_key(&key, b"Test", PREHASHED).is_err());
        assert!(verify_bytes(b"test", PREHASHED).is_err());
    }

    #[test]
    fn legacy_signature_remains_disabled() {
        let key = PublicKey::from_base64(TEST_KEY).unwrap();
        let sig = Signature::decode(LEGACY).unwrap();
        assert!(key.verify(b"test", &sig, true).is_ok());
        assert!(verify_with_key(&key, b"test", LEGACY).is_err());
    }

    #[test]
    fn signature_and_trusted_comment_tampering_are_rejected() {
        let key = PublicKey::from_base64(TEST_KEY).unwrap();
        for line in [1, 3] {
            let changed = mutate_line(line, |bytes| *bytes.last_mut().unwrap() ^= 1);
            assert!(verify_with_key(&key, b"test", &changed).is_err());
        }
        let changed = PREHASHED.replace("file:test", "file:other");
        assert!(verify_with_key(&key, b"test", &changed).is_err());
    }

    #[test]
    fn noncanonical_global_signature_scalar_is_rejected() {
        // Add the Ed25519 group order L to S in the global signature.
        // R and the signed message stay unchanged: a verifier reducing S
        // modulo L would accept this malleated form of the valid signature.
        const ORDER: [u8; 32] = [
            0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9,
            0xde, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
        ];
        let changed = mutate_line(3, |bytes| {
            assert_eq!(bytes.len(), 64);
            let mut carry = 0u16;
            for (byte, order) in bytes[32..].iter_mut().zip(ORDER) {
                let sum = u16::from(*byte) + u16::from(order) + carry;
                *byte = sum as u8;
                carry = sum >> 8;
            }
            assert_eq!(carry, 0);
        });
        let key = PublicKey::from_base64(TEST_KEY).unwrap();
        assert!(verify_with_key(&key, b"test", PREHASHED).is_ok());
        assert!(Signature::decode(&changed).is_ok());
        assert!(verify_with_key(&key, b"test", &changed).is_err());
    }
}
