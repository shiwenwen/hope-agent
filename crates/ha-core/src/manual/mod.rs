//! Built-in bilingual user manual (Help Center backend).
//!
//! Single source of truth is the repo's `docs/user-guide/` tree (zh at the
//! root, en under `en/`), compiled into the binary via `rust-embed` — the
//! same shape as the bundled skills ([`crate::skills`] `embedded.rs`) and the
//! Chrome extension runtime files. Two consumers, two paths:
//!
//! - **GUI**: Tauri / HTTP commands call [`get_bundle`] / [`search`] which
//!   read the embedded bytes in-memory — no disk copy, works in every run
//!   mode including the standalone Web GUI.
//! - **Agent**: the `ha-manual` skill reads/greps the stable on-disk mirror
//!   at `<data-dir>/manual/{zh,en}/NN.md` maintained by
//!   [`ensure_local_manual`] (fingerprint marker + byte-diff mirror, modeled
//!   on the extension's stable-copy machinery).

mod embed;
mod model;
mod search;
mod unpack;

use serde::Serialize;

pub use unpack::ensure_local_manual;

/// A single ATX heading inside a chapter.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ManualHeading {
    /// 1–6.
    pub level: u8,
    /// Heading text with the leading `#`s and trailing closing `#`s stripped.
    pub text: String,
    /// GitHub-style anchor slug (CJK preserved, punctuation dropped,
    /// duplicate slugs suffixed `-1`, `-2`, …). The frontend renders heading
    /// `id`s with the byte-identical algorithm — see `manualSlug.ts`.
    pub slug: String,
    /// 1-based source line.
    pub line: u32,
}

/// One manual chapter (or the README index, `number == 0`).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ManualChapter {
    /// 1..=13 for chapters, 0 for the README index page.
    pub number: u8,
    /// Chapter title from the H1, without the `NN · ` prefix.
    pub title: String,
    /// Full markdown body, verbatim from the source file.
    pub body: String,
    pub headings: Vec<ManualHeading>,
}

/// Everything the Help window needs in one round-trip.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ManualBundle {
    /// The locale that was requested (normalized UI locale).
    pub lang: String,
    /// The manual language actually served (`zh` or `en`).
    pub effective_lang: String,
    /// README index first (number 0), then chapters ascending.
    pub chapters: Vec<ManualChapter>,
}

/// One full-text search hit, at line granularity.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ManualSearchHit {
    pub chapter: u8,
    pub chapter_title: String,
    /// Slug of the nearest heading at or before the hit line (chapter-top
    /// hits have none).
    pub anchor: Option<String>,
    /// 1-based line in the chapter markdown.
    pub line: u32,
    /// The matched line (windowed if long); every term match is wrapped in
    /// STX/ETX (`\u{2}`/`\u{3}`) markers — the contract expected by the
    /// frontend `renderHighlightedSnippet` helper.
    pub snippet: String,
    pub score: i32,
}

/// Map a normalized UI locale to the manual content language. The manual
/// exists in Simplified Chinese and English only; both Chinese variants read
/// far better in zh than in en for native speakers, so zh-TW maps to zh.
pub fn manual_language_for_locale(locale: &str) -> &'static str {
    match crate::i18n::normalize_locale(locale) {
        Some("zh") | Some("zh-TW") => "zh",
        _ => "en",
    }
}

/// Full manual bundle for a UI locale (`lang` may be raw / `"auto"`).
pub fn get_bundle(lang: &str) -> ManualBundle {
    let effective = manual_language_for_locale(lang);
    ManualBundle {
        lang: crate::i18n::normalize_locale(lang)
            .unwrap_or(crate::i18n::DEFAULT_LOCALE)
            .to_string(),
        effective_lang: effective.to_string(),
        chapters: model::chapters(effective),
    }
}

/// Full-text search over the manual in the language for `lang`.
pub fn search(lang: &str, query: &str) -> Vec<ManualSearchHit> {
    let effective = manual_language_for_locale(lang);
    search::search_chapters(&model::chapters(effective), query)
}

/// Command-level bundle entry shared by the Tauri and HTTP shells: resolves
/// the effective UI locale when `lang` is absent, and opportunistically
/// ensures the on-disk mirror (opening the Help window is a natural
/// readiness point for the agent path; the fingerprint marker makes repeat
/// calls zero-IO). Callers already run on the blocking pool.
pub fn bundle_for_command(lang: Option<&str>) -> ManualBundle {
    ensure_local_manual();
    let lang = resolve_command_lang(lang);
    get_bundle(&lang)
}

/// Command-level search entry shared by the Tauri and HTTP shells.
pub fn search_for_command(lang: Option<&str>, query: &str) -> Vec<ManualSearchHit> {
    let lang = resolve_command_lang(lang);
    search(&lang, query)
}

fn resolve_command_lang(lang: Option<&str>) -> String {
    match lang {
        Some(l) if !l.trim().is_empty() => l.to_string(),
        _ => crate::i18n::current_ui_locale().to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn language_mapping_matches_contract() {
        for (locale, want) in [
            ("zh", "zh"),
            ("zh-CN", "zh"),
            ("zh-TW", "zh"), // decided: Traditional readers get Simplified, not English
            ("en", "en"),
            ("ja", "en"),
            ("auto", "en"),
            ("", "en"),
        ] {
            assert_eq!(super::manual_language_for_locale(locale), want, "{locale}");
        }
    }

    /// The ha-manual skill's inline routing table must reference exactly the
    /// chapters that exist — a re-numbered or renamed chapter with a stale
    /// table would silently route the agent to the wrong file.
    #[test]
    fn ha_manual_skill_routing_table_matches_chapters() {
        let skill = include_str!("../../../../skills/ha-manual/SKILL.md");
        let referenced: std::collections::BTreeSet<u8> = regex::Regex::new(r"`(\d{2})\.md`")
            .unwrap()
            .captures_iter(skill)
            .filter_map(|c| c[1].parse().ok())
            .collect();
        let existing: std::collections::BTreeSet<u8> = super::get_bundle("zh")
            .chapters
            .iter()
            .map(|c| c.number)
            .filter(|&n| n != 0)
            .collect();
        assert_eq!(
            referenced, existing,
            "skills/ha-manual/SKILL.md routing table drifted from docs/user-guide chapters"
        );
        assert!(
            skill.contains("`index.md`"),
            "routing table must cover the index page"
        );
    }
}
