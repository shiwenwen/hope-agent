/// Tray menu label translations.
pub(crate) struct TrayLabels {
    pub(crate) show_main: &'static str,
    pub(crate) quick_chat: &'static str,
    pub(crate) new_session: &'static str,
    pub(crate) settings: &'static str,
    pub(crate) quit: &'static str,
}

/// macOS application menu label translations.
pub(crate) struct MacosAppMenuLabels {
    pub(crate) about: &'static str,
    pub(crate) settings: &'static str,
    pub(crate) hide: &'static str,
}

pub(crate) fn tray_labels(lang: &str) -> TrayLabels {
    match lang {
        "zh" | "zh-CN" => TrayLabels {
            show_main: "显示主窗口",
            quick_chat: "快捷对话",
            new_session: "新建对话",
            settings: "设置",
            quit: "退出 Hope Agent",
        },
        "zh-TW" => TrayLabels {
            show_main: "顯示主視窗",
            quick_chat: "快捷對話",
            new_session: "新建對話",
            settings: "設定",
            quit: "退出 Hope Agent",
        },
        "ja" => TrayLabels {
            show_main: "メインウィンドウを表示",
            quick_chat: "クイックチャット",
            new_session: "新しいセッション",
            settings: "設定",
            quit: "Hope Agent を終了",
        },
        "ko" => TrayLabels {
            show_main: "메인 창 표시",
            quick_chat: "빠른 채팅",
            new_session: "새 세션",
            settings: "설정",
            quit: "Hope Agent 종료",
        },
        "es" => TrayLabels {
            show_main: "Mostrar ventana principal",
            quick_chat: "Chat rápido",
            new_session: "Nueva sesión",
            settings: "Configuración",
            quit: "Salir de Hope Agent",
        },
        "pt" => TrayLabels {
            show_main: "Mostrar janela principal",
            quick_chat: "Chat rápido",
            new_session: "Nova sessão",
            settings: "Configurações",
            quit: "Sair do Hope Agent",
        },
        "ru" => TrayLabels {
            show_main: "Показать главное окно",
            quick_chat: "Быстрый чат",
            new_session: "Новый сеанс",
            settings: "Настройки",
            quit: "Выход из Hope Agent",
        },
        "ar" => TrayLabels {
            show_main: "إظهار النافذة الرئيسية",
            quick_chat: "محادثة سريعة",
            new_session: "جلسة جديدة",
            settings: "الإعدادات",
            quit: "إنهاء Hope Agent",
        },
        "tr" => TrayLabels {
            show_main: "Ana pencereyi göster",
            quick_chat: "Hızlı sohbet",
            new_session: "Yeni oturum",
            settings: "Ayarlar",
            quit: "Hope Agent'dan çık",
        },
        "vi" => TrayLabels {
            show_main: "Hiển thị cửa sổ chính",
            quick_chat: "Trò chuyện nhanh",
            new_session: "Phiên mới",
            settings: "Cài đặt",
            quit: "Thoát Hope Agent",
        },
        "ms" => TrayLabels {
            show_main: "Tunjukkan tetingkap utama",
            quick_chat: "Sembang pantas",
            new_session: "Sesi baharu",
            settings: "Tetapan",
            quit: "Keluar Hope Agent",
        },
        _ => TrayLabels {
            show_main: "Show Main Window",
            quick_chat: "Quick Chat",
            new_session: "New Session",
            settings: "Settings",
            quit: "Quit Hope Agent",
        },
    }
}

pub(crate) fn macos_app_menu_labels(lang: &str) -> MacosAppMenuLabels {
    match lang {
        "zh" | "zh-CN" => MacosAppMenuLabels {
            about: "关于 Hope Agent",
            settings: "设置...",
            hide: "隐藏 Hope Agent",
        },
        "zh-TW" => MacosAppMenuLabels {
            about: "關於 Hope Agent",
            settings: "設定...",
            hide: "隱藏 Hope Agent",
        },
        "ja" => MacosAppMenuLabels {
            about: "Hope Agent について",
            settings: "設定...",
            hide: "Hope Agent を非表示",
        },
        "ko" => MacosAppMenuLabels {
            about: "Hope Agent 정보",
            settings: "설정...",
            hide: "Hope Agent 숨기기",
        },
        "es" => MacosAppMenuLabels {
            about: "Acerca de Hope Agent",
            settings: "Configuración...",
            hide: "Ocultar Hope Agent",
        },
        "pt" => MacosAppMenuLabels {
            about: "Sobre o Hope Agent",
            settings: "Configurações...",
            hide: "Ocultar Hope Agent",
        },
        "ru" => MacosAppMenuLabels {
            about: "О Hope Agent",
            settings: "Настройки...",
            hide: "Скрыть Hope Agent",
        },
        "ar" => MacosAppMenuLabels {
            about: "حول Hope Agent",
            settings: "الإعدادات...",
            hide: "إخفاء Hope Agent",
        },
        "tr" => MacosAppMenuLabels {
            about: "Hope Agent Hakkında",
            settings: "Ayarlar...",
            hide: "Hope Agent'ı Gizle",
        },
        "vi" => MacosAppMenuLabels {
            about: "Giới thiệu Hope Agent",
            settings: "Cài đặt...",
            hide: "Ẩn Hope Agent",
        },
        "ms" => MacosAppMenuLabels {
            about: "Perihal Hope Agent",
            settings: "Tetapan...",
            hide: "Sembunyikan Hope Agent",
        },
        _ => MacosAppMenuLabels {
            about: "About Hope Agent",
            settings: "Settings...",
            hide: "Hide Hope Agent",
        },
    }
}

/// Resolve the effective language code. When `"auto"`, detect from the OS locale.
pub(crate) fn resolve_language() -> String {
    let stored = ha_core::config::cached_config().language.clone();

    if stored != "auto" {
        return stored;
    }

    let sys_lang = std::process::Command::new("defaults")
        .args(["read", "NSGlobalDomain", "AppleLanguages"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            s.lines()
                .find(|l| {
                    l.trim().starts_with('"')
                        || (!l.trim().is_empty() && !l.contains('(') && !l.contains(')'))
                })
                .map(|l| {
                    l.trim()
                        .trim_matches(|c: char| c == '"' || c == ',' || c.is_whitespace())
                        .to_string()
                })
        })
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_else(|| "en".to_string());

    let lang_part = sys_lang.split('.').next().unwrap_or("en");
    let lang_part = lang_part.replace('_', "-");

    if lang_part.starts_with("zh-TW") || lang_part.starts_with("zh-Hant") || lang_part == "zh-HK" {
        "zh-TW".to_string()
    } else if lang_part.starts_with("zh") {
        "zh".to_string()
    } else if lang_part.starts_with("ja") {
        "ja".to_string()
    } else if lang_part.starts_with("ko") {
        "ko".to_string()
    } else if lang_part.starts_with("es") {
        "es".to_string()
    } else if lang_part.starts_with("pt") {
        "pt".to_string()
    } else if lang_part.starts_with("ru") {
        "ru".to_string()
    } else if lang_part.starts_with("ar") {
        "ar".to_string()
    } else if lang_part.starts_with("tr") {
        "tr".to_string()
    } else if lang_part.starts_with("vi") {
        "vi".to_string()
    } else if lang_part.starts_with("ms") {
        "ms".to_string()
    } else {
        "en".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{macos_app_menu_labels, tray_labels};

    #[test]
    fn macos_app_menu_labels_follow_simplified_chinese() {
        let labels = macos_app_menu_labels("zh");

        assert_eq!(labels.about, "关于 Hope Agent");
        assert_eq!(labels.settings, "设置...");
        assert_eq!(labels.hide, "隐藏 Hope Agent");
    }

    #[test]
    fn macos_app_menu_labels_fall_back_to_english() {
        let labels = macos_app_menu_labels("fr");

        assert_eq!(labels.about, "About Hope Agent");
        assert_eq!(labels.settings, "Settings...");
        assert_eq!(labels.hide, "Hide Hope Agent");
    }

    #[test]
    fn tray_labels_still_match_existing_english_defaults() {
        let labels = tray_labels("en");

        assert_eq!(labels.show_main, "Show Main Window");
        assert_eq!(labels.quick_chat, "Quick Chat");
        assert_eq!(labels.new_session, "New Session");
        assert_eq!(labels.settings, "Settings");
        assert_eq!(labels.quit, "Quit Hope Agent");
    }
}
