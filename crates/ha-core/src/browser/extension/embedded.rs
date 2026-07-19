//! Chrome extension runtime files embedded into the binary.
//!
//! Mirrors the runtime whitelist previously staged into Tauri resources by
//! `scripts/prepare-chrome-extension.mjs` (now retired). The manifest keeps
//! `key`, so an unpacked install resolves to the fixed dev extension id that
//! the native host `allowed_origins` is derived from. Embedding makes local
//! ("unpacked") install work from every distribution shape — desktop bundles,
//! bare-binary tarballs, headless servers — with no sidecar files, and the
//! stable mirror under the data dir refreshes automatically when the binary
//! (and thus the embedded bytes) changes.

use std::borrow::Cow;

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../extensions/chrome"]
#[include = "manifest.json"]
#[include = "service_worker.js"]
#[include = "popup.html"]
#[include = "popup.js"]
#[include = "icons/*.png"]
#[include = "_locales/*/messages.json"]
struct ExtensionAssets;

/// Sorted `(relative path, bytes)` list of the embedded runtime files.
pub(super) fn extension_files() -> Vec<(String, Vec<u8>)> {
    let mut names: Vec<Cow<'static, str>> = ExtensionAssets::iter().collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|n| ExtensionAssets::get(&n).map(|f| (n.into_owned(), f.data.into_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_runtime_files_with_keyed_manifest() {
        let files = extension_files();
        let manifest = files
            .iter()
            .find(|(rel, _)| rel == "manifest.json")
            .expect("manifest.json embedded");
        let parsed: serde_json::Value = serde_json::from_slice(&manifest.1).unwrap();
        assert!(
            parsed.get("key").is_some(),
            "embedded manifest must keep `key` for the fixed unpacked id"
        );
        assert!(files.iter().any(|(rel, _)| rel == "service_worker.js"));
        assert!(files.iter().any(|(rel, _)| rel.starts_with("icons/")));
        assert!(files
            .iter()
            .any(|(rel, _)| rel.starts_with("_locales/") && rel.ends_with("messages.json")));
        // Whitelist must not leak dev files.
        assert!(!files.iter().any(|(rel, _)| rel.ends_with(".ts")
            || rel.starts_with("test-pages/")
            || rel.starts_with("store-listing/")));
    }
}
