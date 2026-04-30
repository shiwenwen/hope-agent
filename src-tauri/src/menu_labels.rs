/// Tray menu label translations.
pub(crate) struct TrayLabels {
    pub(crate) show_main: &'static str,
    pub(crate) quick_chat: &'static str,
    pub(crate) new_session: &'static str,
    pub(crate) settings: &'static str,
    pub(crate) quit: &'static str,
    pub(crate) quit_confirm_title: &'static str,
    pub(crate) quit_confirm_body: &'static str,
    pub(crate) quit_confirm_ok: &'static str,
    pub(crate) quit_confirm_cancel: &'static str,
}

/// Disabled status rows shown at the top of the tray menu.
#[derive(Clone, Copy)]
pub(crate) struct TrayStatusLabels {
    pub(crate) runtime_status: &'static str,
    pub(crate) bound_addr: &'static str,
    pub(crate) uptime: &'static str,
    pub(crate) active_connections: &'static str,
    pub(crate) active_sessions: &'static str,
    pub(crate) startup_error: &'static str,
    pub(crate) not_started: &'static str,
    pub(crate) event_unit: &'static str,
    pub(crate) chat_unit: &'static str,
    pub(crate) local_unit: &'static str,
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
            quit_confirm_title: "退出 Hope Agent？",
            quit_confirm_body: "退出后会停止所有后台任务（IM 渠道、定时任务、流式对话）。确定退出？",
            quit_confirm_ok: "退出",
            quit_confirm_cancel: "取消",
        },
        "zh-TW" => TrayLabels {
            show_main: "顯示主視窗",
            quick_chat: "快捷對話",
            new_session: "新建對話",
            settings: "設定",
            quit: "退出 Hope Agent",
            quit_confirm_title: "退出 Hope Agent？",
            quit_confirm_body: "退出後將停止所有後台任務（IM 渠道、定時任務、串流對話）。確定退出？",
            quit_confirm_ok: "退出",
            quit_confirm_cancel: "取消",
        },
        "ja" => TrayLabels {
            show_main: "メインウィンドウを表示",
            quick_chat: "クイックチャット",
            new_session: "新しいセッション",
            settings: "設定",
            quit: "Hope Agent を終了",
            quit_confirm_title: "Hope Agent を終了しますか？",
            quit_confirm_body: "終了するとすべてのバックグラウンドタスク（IM チャネル、スケジュールジョブ、ストリーミングチャット）が停止します。続行しますか？",
            quit_confirm_ok: "終了",
            quit_confirm_cancel: "キャンセル",
        },
        "ko" => TrayLabels {
            show_main: "메인 창 표시",
            quick_chat: "빠른 채팅",
            new_session: "새 세션",
            settings: "설정",
            quit: "Hope Agent 종료",
            quit_confirm_title: "Hope Agent을 종료하시겠습니까?",
            quit_confirm_body: "종료하면 모든 백그라운드 작업(IM 채널, 예약 작업, 스트리밍 채팅)이 중지됩니다. 계속하시겠습니까?",
            quit_confirm_ok: "종료",
            quit_confirm_cancel: "취소",
        },
        "es" => TrayLabels {
            show_main: "Mostrar ventana principal",
            quick_chat: "Chat rápido",
            new_session: "Nueva sesión",
            settings: "Configuración",
            quit: "Salir de Hope Agent",
            quit_confirm_title: "¿Salir de Hope Agent?",
            quit_confirm_body: "Al salir se detendrán todas las tareas en segundo plano (canales IM, tareas programadas, chats en streaming). ¿Continuar?",
            quit_confirm_ok: "Salir",
            quit_confirm_cancel: "Cancelar",
        },
        "pt" => TrayLabels {
            show_main: "Mostrar janela principal",
            quick_chat: "Chat rápido",
            new_session: "Nova sessão",
            settings: "Configurações",
            quit: "Sair do Hope Agent",
            quit_confirm_title: "Sair do Hope Agent?",
            quit_confirm_body: "Sair encerrará todas as tarefas em segundo plano (canais IM, tarefas agendadas, chats em streaming). Continuar?",
            quit_confirm_ok: "Sair",
            quit_confirm_cancel: "Cancelar",
        },
        "ru" => TrayLabels {
            show_main: "Показать главное окно",
            quick_chat: "Быстрый чат",
            new_session: "Новый сеанс",
            settings: "Настройки",
            quit: "Выход из Hope Agent",
            quit_confirm_title: "Выйти из Hope Agent?",
            quit_confirm_body: "Выход остановит все фоновые задачи (каналы IM, запланированные задания, потоковые чаты). Продолжить?",
            quit_confirm_ok: "Выйти",
            quit_confirm_cancel: "Отмена",
        },
        "ar" => TrayLabels {
            show_main: "إظهار النافذة الرئيسية",
            quick_chat: "محادثة سريعة",
            new_session: "جلسة جديدة",
            settings: "الإعدادات",
            quit: "إنهاء Hope Agent",
            quit_confirm_title: "إنهاء Hope Agent؟",
            quit_confirm_body: "سيؤدي الإنهاء إلى إيقاف جميع المهام في الخلفية (قنوات IM، المهام المجدولة، المحادثات المتدفقة). هل تريد المتابعة؟",
            quit_confirm_ok: "إنهاء",
            quit_confirm_cancel: "إلغاء",
        },
        "tr" => TrayLabels {
            show_main: "Ana pencereyi göster",
            quick_chat: "Hızlı sohbet",
            new_session: "Yeni oturum",
            settings: "Ayarlar",
            quit: "Hope Agent'dan çık",
            quit_confirm_title: "Hope Agent'tan çıkılsın mı?",
            quit_confirm_body: "Çıkış tüm arka plan görevlerini (IM kanalları, zamanlanmış işler, akışlı sohbetler) durduracak. Devam edilsin mi?",
            quit_confirm_ok: "Çık",
            quit_confirm_cancel: "İptal",
        },
        "vi" => TrayLabels {
            show_main: "Hiển thị cửa sổ chính",
            quick_chat: "Trò chuyện nhanh",
            new_session: "Phiên mới",
            settings: "Cài đặt",
            quit: "Thoát Hope Agent",
            quit_confirm_title: "Thoát Hope Agent?",
            quit_confirm_body: "Thoát sẽ dừng tất cả tác vụ chạy nền (kênh IM, tác vụ đã lên lịch, trò chuyện trực tuyến). Tiếp tục?",
            quit_confirm_ok: "Thoát",
            quit_confirm_cancel: "Hủy",
        },
        "ms" => TrayLabels {
            show_main: "Tunjukkan tetingkap utama",
            quick_chat: "Sembang pantas",
            new_session: "Sesi baharu",
            settings: "Tetapan",
            quit: "Keluar Hope Agent",
            quit_confirm_title: "Keluar Hope Agent?",
            quit_confirm_body: "Keluar akan menghentikan semua tugasan latar (saluran IM, tugasan berjadual, sembang penstriman). Teruskan?",
            quit_confirm_ok: "Keluar",
            quit_confirm_cancel: "Batal",
        },
        _ => TrayLabels {
            show_main: "Show Main Window",
            quick_chat: "Quick Chat",
            new_session: "New Session",
            settings: "Settings",
            quit: "Quit Hope Agent",
            quit_confirm_title: "Quit Hope Agent?",
            quit_confirm_body: "Quitting stops all background tasks (IM channels, scheduled jobs, chat streams). Continue?",
            quit_confirm_ok: "Quit",
            quit_confirm_cancel: "Cancel",
        },
    }
}

pub(crate) fn tray_status_labels(lang: &str) -> TrayStatusLabels {
    match lang {
        "zh" | "zh-CN" => TrayStatusLabels {
            runtime_status: "运行时状态",
            bound_addr: "绑定地址",
            uptime: "运行时长",
            active_connections: "活跃连接",
            active_sessions: "活跃会话",
            startup_error: "启动错误",
            not_started: "未启动",
            event_unit: "事件",
            chat_unit: "会话",
            local_unit: "本机",
        },
        "zh-TW" => TrayStatusLabels {
            runtime_status: "執行時狀態",
            bound_addr: "綁定地址",
            uptime: "運行時長",
            active_connections: "活躍連接",
            active_sessions: "活躍對話",
            startup_error: "啟動錯誤",
            not_started: "未啟動",
            event_unit: "事件",
            chat_unit: "對話",
            local_unit: "本機",
        },
        "ja" => TrayStatusLabels {
            runtime_status: "ランタイムステータス",
            bound_addr: "バインドアドレス",
            uptime: "稼働時間",
            active_connections: "アクティブな WebSocket",
            active_sessions: "アクティブなチャット",
            startup_error: "起動エラー",
            not_started: "未起動",
            event_unit: "events",
            chat_unit: "chat",
            local_unit: "ローカル",
        },
        "ko" => TrayStatusLabels {
            runtime_status: "런타임 상태",
            bound_addr: "바인딩 주소",
            uptime: "가동 시간",
            active_connections: "활성 WebSocket",
            active_sessions: "활성 채팅 세션",
            startup_error: "시작 오류",
            not_started: "시작되지 않음",
            event_unit: "events",
            chat_unit: "chat",
            local_unit: "로컬",
        },
        "es" => TrayStatusLabels {
            runtime_status: "Estado en tiempo real",
            bound_addr: "Dirección vinculada",
            uptime: "Tiempo activo",
            active_connections: "WebSockets activos",
            active_sessions: "Sesiones de chat activas",
            startup_error: "Error de inicio",
            not_started: "No iniciado",
            event_unit: "eventos",
            chat_unit: "chat",
            local_unit: "local",
        },
        "pt" => TrayStatusLabels {
            runtime_status: "Estado em tempo de execução",
            bound_addr: "Endereço vinculado",
            uptime: "Tempo ativo",
            active_connections: "WebSockets ativos",
            active_sessions: "Sessões de chat ativas",
            startup_error: "Erro de inicialização",
            not_started: "Não iniciado",
            event_unit: "eventos",
            chat_unit: "chat",
            local_unit: "local",
        },
        "ru" => TrayStatusLabels {
            runtime_status: "Состояние во время работы",
            bound_addr: "Привязанный адрес",
            uptime: "Время работы",
            active_connections: "Активные WebSocket",
            active_sessions: "Активные чат-сессии",
            startup_error: "Ошибка запуска",
            not_started: "Не запущено",
            event_unit: "событий",
            chat_unit: "чат",
            local_unit: "локально",
        },
        "ar" => TrayStatusLabels {
            runtime_status: "حالة التشغيل",
            bound_addr: "العنوان المُرتبط",
            uptime: "مدة التشغيل",
            active_connections: "اتصالات WebSocket النشطة",
            active_sessions: "جلسات المحادثة النشطة",
            startup_error: "خطأ في بدء التشغيل",
            not_started: "لم يبدأ",
            event_unit: "أحداث",
            chat_unit: "محادثة",
            local_unit: "محلي",
        },
        "tr" => TrayStatusLabels {
            runtime_status: "Çalışma Durumu",
            bound_addr: "Bağlı Adres",
            uptime: "Çalışma Süresi",
            active_connections: "Aktif WebSocket",
            active_sessions: "Aktif Sohbet Oturumları",
            startup_error: "Başlatma Hatası",
            not_started: "Başlatılmadı",
            event_unit: "olay",
            chat_unit: "sohbet",
            local_unit: "yerel",
        },
        "vi" => TrayStatusLabels {
            runtime_status: "Trạng thái hoạt động",
            bound_addr: "Địa chỉ liên kết",
            uptime: "Thời gian hoạt động",
            active_connections: "WebSocket đang hoạt động",
            active_sessions: "Phiên trò chuyện đang hoạt động",
            startup_error: "Lỗi khởi động",
            not_started: "Chưa khởi động",
            event_unit: "sự kiện",
            chat_unit: "trò chuyện",
            local_unit: "cục bộ",
        },
        "ms" => TrayStatusLabels {
            runtime_status: "Status Jalanan",
            bound_addr: "Alamat Terikat",
            uptime: "Masa Beroperasi",
            active_connections: "WebSocket Aktif",
            active_sessions: "Sesi Sembang Aktif",
            startup_error: "Ralat Permulaan",
            not_started: "Belum bermula",
            event_unit: "peristiwa",
            chat_unit: "sembang",
            local_unit: "tempatan",
        },
        _ => TrayStatusLabels {
            runtime_status: "Runtime Status",
            bound_addr: "Bound Address",
            uptime: "Uptime",
            active_connections: "Active WebSockets",
            active_sessions: "Active Chat Streams",
            startup_error: "Startup Error",
            not_started: "Not started",
            event_unit: "events",
            chat_unit: "chat",
            local_unit: "local",
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
