//! Prompt template shown to the user when importing memories from another AI assistant.
//!
//! The user copies this prompt into an external AI (ChatGPT / Claude / Gemini / ...),
//! lets it summarize what it knows about them as a JSON array, and pastes the result
//! back into the import dialog. The backend then feeds the pasted text through the
//! existing [`crate::memory::parse_import_json`] pipeline.
//!
//! Templates live in `crates/oc-core/templates/memory_import_from_ai.<locale>.md`
//! and are embedded at compile time via `include_str!`. Currently only `en` and `zh`
//! have hand-written templates; all other locales fall back to English. To add another
//! language, drop a new `.md` file next to the existing ones and add a match arm below.

/// Return the "import memory from another AI" prompt template for the given locale.
///
/// The locale string follows the same convention as [`crate::agent_loader::default_agent_md`]
/// (e.g. `"en"`, `"zh"`, `"zh-TW"`, `"ja"`, ...). Unknown or unsupported locales fall
/// back to English.
pub fn import_from_ai_prompt(locale: &str) -> &'static str {
    match locale {
        "zh" => include_str!("../../templates/memory_import_from_ai.zh.md"),
        _ => include_str!("../../templates/memory_import_from_ai.en.md"),
    }
}
