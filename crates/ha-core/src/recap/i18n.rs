//! Output-language resolution and section-title localization for recap reports.
//!
//! The recap pipeline drives its LLM prompts in the user's language and writes
//! localized section titles into the persisted report (a language snapshot), so
//! both the Dashboard view and the exported HTML render in one consistent
//! language. Facet/section *prose* is produced by the LLM via a language
//! directive built from [`language_name`]; section *titles* are fixed strings
//! translated here.

use crate::config::AppConfig;

/// The recap output locales we have full title/name coverage for. Column order
/// here is the single source of truth for [`locale_index`] and therefore for
/// the per-section title rows in [`localized_section_title`].
pub(super) const SUPPORTED_LOCALES: [&str; 12] = [
    "zh", "zh-TW", "en", "ja", "ko", "es", "pt", "ru", "ar", "tr", "vi", "ms",
];

/// Resolve the effective output locale for recap generation.
///
/// Precedence: explicit `recap.language` > global `AppConfig.language` >
/// system locale. Empty / `"auto"` at any level falls through to the next.
pub(super) fn effective_recap_locale(config: &AppConfig) -> String {
    fn pick(s: &str) -> Option<String> {
        let t = s.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(t.to_string())
        }
    }
    let resolved = config
        .recap
        .language
        .as_deref()
        .and_then(pick)
        .or_else(|| pick(&config.language))
        .unwrap_or_else(crate::agent_loader::detect_system_locale);
    // Normalize to a supported locale — `detect_system_locale` returns the first
    // 2 chars of the OS locale, which can be a code we have no coverage for
    // (e.g. "de"/"fr"). Fall back to English rather than emit a contradictory
    // "write in English" directive while titles silently use the English column.
    // Case-insensitive match back to the canonical code; non-GUI paths
    // (ha-settings skill / manual config edits) may write e.g. "zh-tw" or "ZH",
    // which must still resolve to the canonical "zh-TW"/"zh" rather than English.
    SUPPORTED_LOCALES
        .iter()
        .find(|c| c.eq_ignore_ascii_case(&resolved))
        .map(|c| c.to_string())
        .unwrap_or_else(|| "en".to_string())
}

/// Human-readable language name (with native script hint) for a locale code,
/// used to instruct the LLM which language to write in. Unknown codes fall
/// back to English.
pub(super) fn language_name(locale: &str) -> &'static str {
    match locale {
        "zh" => "Simplified Chinese (简体中文)",
        "zh-TW" => "Traditional Chinese (繁體中文)",
        "ja" => "Japanese (日本語)",
        "ko" => "Korean (한국어)",
        "es" => "Spanish (Español)",
        "pt" => "Portuguese (Português)",
        "ru" => "Russian (Русский)",
        "ar" => "Arabic (العربية)",
        "tr" => "Turkish (Türkçe)",
        "vi" => "Vietnamese (Tiếng Việt)",
        "ms" => "Malay (Bahasa Melayu)",
        _ => "English",
    }
}

/// Localized report list title, e.g. `复盘 2024-01-01 → 2024-02-01 (12 个会话)`.
/// Uses an invariant `count + word` form to avoid per-language plural rules.
pub(super) fn report_title(locale: &str, start: &str, end: &str, sessions: u32) -> String {
    let (prefix, word) = match locale {
        "zh" => ("复盘", "个会话"),
        "zh-TW" => ("回顧", "個對話"),
        "ja" => ("振り返り", "セッション"),
        "ko" => ("돌아보기", "세션"),
        "es" => ("Resumen", "sesiones"),
        "pt" => ("Retrospectiva", "sessões"),
        "ru" => ("Обзор", "сессий"),
        "ar" => ("مراجعة", "جلسة"),
        "tr" => ("Özet", "oturum"),
        "vi" => ("Tổng kết", "phiên"),
        "ms" => ("Imbasan", "sesi"),
        _ => ("Recap", "sessions"),
    };
    format!("{prefix} {start} → {end} ({sessions} {word})")
}

/// Column index into the per-section translation rows below. Order must match
/// the literal arrays in [`localized_section_title`]. English at index 2 is the
/// fallback for unknown locales.
fn locale_index(locale: &str) -> usize {
    // Single source of truth: position in SUPPORTED_LOCALES. Unknown → English.
    SUPPORTED_LOCALES
        .iter()
        .position(|&l| l == locale)
        .unwrap_or(2)
}

/// Localized title for a recap section `key`. Rows are ordered
/// `[zh, zh-TW, en, ja, ko, es, pt, ru, ar, tr, vi, ms]`. Unknown keys fall
/// back to the English title; unknown locales fall back to the English column.
pub(super) fn localized_section_title(key: &str, locale: &str) -> &'static str {
    let row: [&'static str; 12] = match key {
        "project_areas" => [
            "你的工作领域",
            "你的工作領域",
            "What you work on",
            "取り組んでいる領域",
            "작업 중인 영역",
            "En qué trabajas",
            "No que você trabalha",
            "Над чем вы работаете",
            "مجالات عملك",
            "Üzerinde çalıştıklarınız",
            "Lĩnh vực bạn làm việc",
            "Bidang kerja anda",
        ],
        "interaction_style" => [
            "你如何使用 Hope Agent",
            "你如何使用 Hope Agent",
            "How you use Hope Agent",
            "Hope Agent の使い方",
            "Hope Agent 사용 방식",
            "Cómo usas Hope Agent",
            "Como você usa o Hope Agent",
            "Как вы используете Hope Agent",
            "كيف تستخدم Hope Agent",
            "Hope Agent'ı nasıl kullanıyorsunuz",
            "Cách bạn dùng Hope Agent",
            "Cara anda guna Hope Agent",
        ],
        "what_works" => [
            "哪些做得好",
            "哪些做得好",
            "What's working well",
            "うまくいっていること",
            "잘 되고 있는 점",
            "Lo que funciona bien",
            "O que está funcionando bem",
            "Что работает хорошо",
            "ما الذي ينجح",
            "Neler iyi gidiyor",
            "Điều đang hiệu quả",
            "Apa yang berkesan",
        ],
        "friction_analysis" => [
            "卡点在哪",
            "卡點在哪",
            "Where things get stuck",
            "詰まりやすいところ",
            "막히는 지점",
            "Dónde te atascas",
            "Onde as coisas travam",
            "Где возникают затруднения",
            "أين تتعثر الأمور",
            "Nerede tıkanıyorsunuz",
            "Chỗ hay bị tắc",
            "Di mana tersekat",
        ],
        "agent_tool_optimization" => [
            "智能体与工具优化",
            "智能體與工具最佳化",
            "Agent & tool optimization",
            "エージェントとツールの最適化",
            "에이전트 및 도구 최적화",
            "Optimización de agentes y herramientas",
            "Otimização de agentes e ferramentas",
            "Оптимизация агентов и инструментов",
            "تحسين الوكيل والأدوات",
            "Aracı ve araç optimizasyonu",
            "Tối ưu agent & công cụ",
            "Pengoptimuman ejen & alat",
        ],
        "memory_skill_recommendations" => [
            "记忆与技能建议",
            "記憶與技能建議",
            "Memory & skill recommendations",
            "メモリとスキルの推奨",
            "메모리 및 스킬 추천",
            "Recomendaciones de memoria y habilidades",
            "Recomendações de memória e habilidades",
            "Рекомендации по памяти и навыкам",
            "توصيات الذاكرة والمهارات",
            "Bellek ve beceri önerileri",
            "Đề xuất bộ nhớ & kỹ năng",
            "Cadangan memori & kemahiran",
        ],
        "cost_optimization" => [
            "成本优化",
            "成本最佳化",
            "Cost optimization",
            "コスト最適化",
            "비용 최적화",
            "Optimización de costos",
            "Otimização de custos",
            "Оптимизация затрат",
            "تحسين التكلفة",
            "Maliyet optimizasyonu",
            "Tối ưu chi phí",
            "Pengoptimuman kos",
        ],
        "suggestions" => [
            "建议",
            "建議",
            "Suggestions",
            "提案",
            "제안",
            "Sugerencias",
            "Sugestões",
            "Предложения",
            "اقتراحات",
            "Öneriler",
            "Gợi ý",
            "Cadangan",
        ],
        "on_the_horizon" => [
            "未来可期",
            "未來可期",
            "On the horizon",
            "これからの展望",
            "앞으로의 전망",
            "En el horizonte",
            "No horizonte",
            "На горизонте",
            "في الأفق",
            "Ufukta",
            "Triển vọng sắp tới",
            "Di kaki langit",
        ],
        "fun_ending" => [
            "难忘瞬间",
            "難忘瞬間",
            "Memorable moment",
            "思い出に残る瞬間",
            "기억에 남는 순간",
            "Momento memorable",
            "Momento memorável",
            "Запоминающийся момент",
            "لحظة لا تُنسى",
            "Unutulmaz an",
            "Khoảnh khắc đáng nhớ",
            "Detik tak dilupakan",
        ],
        "at_a_glance" => [
            "一览",
            "一覽",
            "At a glance",
            "ひと目で",
            "한눈에 보기",
            "De un vistazo",
            "Visão geral",
            "Кратко",
            "لمحة سريعة",
            "Bir bakışta",
            "Tổng quan nhanh",
            "Sekilas pandang",
        ],
        _ => return "",
    };
    row[locale_index(locale)]
}

/// Language directive for facet-extraction prompts: natural-language fields
/// follow the locale; enum / JSON-key / category tokens stay English so
/// aggregation stays stable. Empty for English.
pub(super) fn facet_language_directive(locale: &str) -> String {
    if locale.eq_ignore_ascii_case("en") {
        return String::new();
    }
    format!(
        "IMPORTANT: Write every natural-language string value (underlyingGoal, frictionDetail, \
         primarySuccess, briefSummary, userInstructions) in {lang}. Keep all JSON keys and the \
         outcome / sessionType / goalCategories token values in English exactly as specified.",
        lang = language_name(locale)
    )
}

/// Language directive for report-section prose. Empty for English. Keeps code
/// identifiers, model names, paths and Hope Agent command names untranslated.
pub(super) fn section_language_directive(locale: &str) -> String {
    if locale.eq_ignore_ascii_case("en") {
        return String::new();
    }
    format!(
        "IMPORTANT: Write the entire section (all prose, headings, and bullet labels) in {lang}. \
         Keep code identifiers, model names, file paths, and Hope Agent command names (e.g. \
         /remember) unchanged.",
        lang = language_name(locale)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECTION_KEYS: [&str; 11] = [
        "project_areas",
        "interaction_style",
        "what_works",
        "friction_analysis",
        "agent_tool_optimization",
        "memory_skill_recommendations",
        "cost_optimization",
        "suggestions",
        "on_the_horizon",
        "fun_ending",
        "at_a_glance",
    ];

    #[test]
    fn every_supported_locale_is_fully_covered() {
        for &loc in &SUPPORTED_LOCALES {
            assert_ne!(language_name(loc), "", "language_name empty for {loc}");
            assert!(
                !report_title(loc, "a", "b", 1).is_empty(),
                "report_title empty for {loc}"
            );
            for key in SECTION_KEYS {
                assert!(
                    !localized_section_title(key, loc).is_empty(),
                    "section title empty for {key}/{loc}"
                );
            }
        }
    }

    #[test]
    fn locale_columns_are_not_misaligned() {
        // Anchor every column of one row so a swap/insert of ANY two columns
        // (including non-adjacent mid-array) fails fast — length alone is
        // guarded by the [&str; 12] type, but per-column correctness is not.
        let expected = [
            ("zh", "你的工作领域"),
            ("zh-TW", "你的工作領域"),
            ("en", "What you work on"),
            ("ja", "取り組んでいる領域"),
            ("ko", "작업 중인 영역"),
            ("es", "En qué trabajas"),
            ("pt", "No que você trabalha"),
            ("ru", "Над чем вы работаете"),
            ("ar", "مجالات عملك"),
            ("tr", "Üzerinde çalıştıklarınız"),
            ("vi", "Lĩnh vực bạn làm việc"),
            ("ms", "Bidang kerja anda"),
        ];
        for (loc, title) in expected {
            assert_eq!(
                localized_section_title("project_areas", loc),
                title,
                "{loc}"
            );
        }
        assert_eq!(localized_section_title("at_a_glance", "ja"), "ひと目で");
        assert_eq!(locale_index("en"), 2);
        assert_eq!(SUPPORTED_LOCALES[locale_index("ms")], "ms");
    }

    #[test]
    fn unknown_locale_falls_back_to_english() {
        assert_eq!(locale_index("de"), 2);
        assert_eq!(
            localized_section_title("project_areas", "de"),
            "What you work on"
        );
        assert_eq!(language_name("de"), "English");
    }

    #[test]
    fn effective_locale_normalizes_case_and_unsupported() {
        let mut cfg = AppConfig::default();
        cfg.recap.language = Some("zh-tw".to_string());
        assert_eq!(effective_recap_locale(&cfg), "zh-TW");
        cfg.recap.language = Some("ZH".to_string());
        assert_eq!(effective_recap_locale(&cfg), "zh");
        cfg.recap.language = Some("de".to_string());
        assert_eq!(effective_recap_locale(&cfg), "en");
    }
}
