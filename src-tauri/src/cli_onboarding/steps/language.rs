//! Step 1 — language selection.

use anyhow::Result;

use ha_core::onboarding::apply;

use crate::cli_onboarding::prompt::{print_saved, println_step, prompt_select};

/// The full list of supported language codes, kept in sync with the
/// `SUPPORTED_LANGUAGES` array in `src/i18n/i18n.ts`.
const LANGUAGES: &[(&str, &str)] = &[
    ("auto", "Follow system"),
    ("zh", "简体中文"),
    ("zh-TW", "繁體中文"),
    ("en", "English"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("tr", "Türkçe"),
    ("vi", "Tiếng Việt"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("ar", "العربية"),
    ("es", "Español"),
    ("ms", "Bahasa Melayu"),
];

pub fn run(step: u32, total: u32) -> Result<()> {
    println_step(step, total, "Language / 语言");
    let labels: Vec<&str> = LANGUAGES.iter().map(|(_, label)| *label).collect();
    let idx = prompt_select("Select a display language:", &labels, 0)?;
    let code = LANGUAGES[idx].0;
    apply::apply_language(code)?;
    print_saved(&format!("Language set to {}", LANGUAGES[idx].1));
    Ok(())
}
