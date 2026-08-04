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
    }

    #[test]
    fn embeds_exact_runtime_file_set() {
        // Successor to the retired prepare-chrome-extension.mjs fail-fast
        // whitelist: rust-embed `#[include]` silently embeds nothing for a
        // missing/renamed file, so this exact-set assertion is what turns
        // that into a CI failure instead of a silently broken release
        // extension (dead popup, refused load on a missing manifest-referenced
        // icon/locale). Update it in lockstep with the runtime file set.
        const LOCALES: [&str; 12] = [
            "ar", "en", "es", "ja", "ko", "ms", "pt", "ru", "tr", "vi", "zh_CN", "zh_TW",
        ];
        let mut want: Vec<String> = [
            "manifest.json",
            "service_worker.js",
            "popup.html",
            "popup.js",
            "icons/icon16.png",
            "icons/icon32.png",
            "icons/icon48.png",
            "icons/icon128.png",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        want.extend(
            LOCALES
                .iter()
                .map(|l| format!("_locales/{l}/messages.json")),
        );
        want.sort();
        let got: Vec<String> = extension_files().into_iter().map(|(rel, _)| rel).collect();
        assert_eq!(
            got, want,
            "embedded runtime set drifted from the expected whitelist"
        );
    }
}
