//! The `docs/user-guide/` tree embedded into the binary.
//!
//! Release builds bake the files in; debug builds read the workspace
//! directory at call time (rust-embed default), so manual edits show up in
//! the running dev app without a rebuild. `build.rs` declares
//! `rerun-if-changed=../../docs/user-guide` so added/removed chapters
//! invalidate warm release rebuilds.

use std::borrow::Cow;

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../docs/user-guide"]
struct ManualAssets;

/// Sorted `(relative path, bytes)` list of every embedded manual file.
///
/// Keys come from `iter()` verbatim — the zh filenames are non-ASCII
/// (`02-模型与Provider.md`) and macOS/Linux checkouts can differ in NFC/NFD
/// normalization, so callers must never construct a lookup key themselves;
/// chapters are identified by the ASCII `NN` filename prefix and the `en/`
/// path prefix only (see `model.rs`).
pub(super) fn manual_files() -> Vec<(String, Cow<'static, [u8]>)> {
    let mut names: Vec<_> = ManualAssets::iter().collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|n| ManualAssets::get(&n).map(|f| (n.into_owned(), f.data)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hard gate that the embed is populated: a build environment that lacks
    /// `docs/user-guide` (e.g. a Docker stage missing the COPY) would
    /// otherwise compile green and ship an empty manual.
    #[test]
    fn embeds_the_full_bilingual_manual() {
        let files = manual_files();
        let zh = files
            .iter()
            .filter(|(n, _)| !n.starts_with("en/") && n.ends_with(".md"))
            .count();
        let en = files
            .iter()
            .filter(|(n, _)| n.starts_with("en/") && n.ends_with(".md"))
            .count();
        // 13 chapters + README per language. Growth is fine; emptiness or a
        // one-sided tree is not.
        assert!(zh >= 14, "zh manual incomplete: {zh} files");
        assert!(en >= 14, "en manual incomplete: {en} files");
        assert_eq!(zh, en, "zh/en chapter counts diverged");
        for (name, data) in &files {
            assert!(!data.is_empty(), "embedded manual file {name} is empty");
        }
    }
}
