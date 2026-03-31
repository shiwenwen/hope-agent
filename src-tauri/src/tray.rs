use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};

/// Show and focus the main window if it already exists.
fn show_main_window(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Tray menu label translations.
struct TrayLabels {
    show_main: &'static str,
    quick_chat: &'static str,
    new_session: &'static str,
    settings: &'static str,
    quit: &'static str,
}

fn tray_labels(lang: &str) -> TrayLabels {
    match lang {
        "zh" | "zh-CN" => TrayLabels {
            show_main: "显示主窗口",
            quick_chat: "快捷对话",
            new_session: "新建对话",
            settings: "设置",
            quit: "退出 OpenComputer",
        },
        "zh-TW" => TrayLabels {
            show_main: "顯示主視窗",
            quick_chat: "快捷對話",
            new_session: "新建對話",
            settings: "設定",
            quit: "退出 OpenComputer",
        },
        "ja" => TrayLabels {
            show_main: "メインウィンドウを表示",
            quick_chat: "クイックチャット",
            new_session: "新しいセッション",
            settings: "設定",
            quit: "OpenComputer を終了",
        },
        "ko" => TrayLabels {
            show_main: "메인 창 표시",
            quick_chat: "빠른 채팅",
            new_session: "새 세션",
            settings: "설정",
            quit: "OpenComputer 종료",
        },
        "es" => TrayLabels {
            show_main: "Mostrar ventana principal",
            quick_chat: "Chat rápido",
            new_session: "Nueva sesión",
            settings: "Configuración",
            quit: "Salir de OpenComputer",
        },
        "pt" => TrayLabels {
            show_main: "Mostrar janela principal",
            quick_chat: "Chat rápido",
            new_session: "Nova sessão",
            settings: "Configurações",
            quit: "Sair do OpenComputer",
        },
        "ru" => TrayLabels {
            show_main: "Показать главное окно",
            quick_chat: "Быстрый чат",
            new_session: "Новый сеанс",
            settings: "Настройки",
            quit: "Выход из OpenComputer",
        },
        "ar" => TrayLabels {
            show_main: "إظهار النافذة الرئيسية",
            quick_chat: "محادثة سريعة",
            new_session: "جلسة جديدة",
            settings: "الإعدادات",
            quit: "إنهاء OpenComputer",
        },
        "tr" => TrayLabels {
            show_main: "Ana pencereyi göster",
            quick_chat: "Hızlı sohbet",
            new_session: "Yeni oturum",
            settings: "Ayarlar",
            quit: "OpenComputer'dan çık",
        },
        "vi" => TrayLabels {
            show_main: "Hiển thị cửa sổ chính",
            quick_chat: "Trò chuyện nhanh",
            new_session: "Phiên mới",
            settings: "Cài đặt",
            quit: "Thoát OpenComputer",
        },
        "ms" => TrayLabels {
            show_main: "Tunjukkan tetingkap utama",
            quick_chat: "Sembang pantas",
            new_session: "Sesi baharu",
            settings: "Tetapan",
            quit: "Keluar OpenComputer",
        },
        // English (default)
        _ => TrayLabels {
            show_main: "Show Main Window",
            quick_chat: "Quick Chat",
            new_session: "New Session",
            settings: "Settings",
            quit: "Quit OpenComputer",
        },
    }
}

/// Resolve the effective language code. When `"auto"`, detect from macOS system locale.
fn resolve_language() -> String {
    let stored = crate::provider::load_store()
        .map(|s| s.language)
        .unwrap_or_else(|_| "auto".to_string());

    if stored != "auto" {
        return stored;
    }

    // Detect system language: macOS uses AppleLanguages (e.g. "zh-Hans-CN"),
    // fall back to LANG env var (e.g. "zh_CN.UTF-8")
    let sys_lang = std::process::Command::new("defaults")
        .args(["read", "NSGlobalDomain", "AppleLanguages"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            // Parse plist array: extract first entry like "zh-Hans-CN"
            s.lines()
                .find(|l| {
                    l.trim().starts_with('"')
                        || (l.trim().len() > 0 && !l.contains('(') && !l.contains(')'))
                })
                .map(|l| {
                    l.trim()
                        .trim_matches(|c: char| c == '"' || c == ',' || c.is_whitespace())
                        .to_string()
                })
        })
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_else(|| "en".to_string());

    // Normalize: "zh-Hans-CN" / "zh_CN.UTF-8" → comparable form
    let lang_part = sys_lang.split('.').next().unwrap_or("en");
    let lang_part = lang_part.replace('_', "-");

    // Map to supported language codes
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

/// Set up the system tray icon with context menu.
pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let lang = resolve_language();
    let labels = tray_labels(&lang);

    // Build menu items
    let show_main = MenuItemBuilder::with_id("show_main", labels.show_main).build(app)?;
    let quick_chat = MenuItemBuilder::with_id("quick_chat", labels.quick_chat).build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let new_session = MenuItemBuilder::with_id("new_session", labels.new_session).build(app)?;
    let open_settings = MenuItemBuilder::with_id("open_settings", labels.settings).build(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_app = MenuItemBuilder::with_id("quit_app", labels.quit).build(app)?;

    let menu = MenuBuilder::new(app)
        .items(&[
            &show_main,
            &quick_chat,
            &sep1,
            &new_session,
            &open_settings,
            &sep2,
            &quit_app,
        ])
        .build()?;

    // Read icon config from tauri.conf.json trayIcon section.
    let (icon, icon_as_template, show_menu_on_left_click) = match &app.config().app.tray_icon {
        Some(tc) => {
            let icon_path =
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&tc.icon_path);
            let img = tauri::image::Image::from_path(&icon_path).unwrap_or_else(|_| {
                tauri::image::Image::from_bytes(include_bytes!("../icons/menuIconTray.png"))
                    .unwrap()
            });
            (img, tc.icon_as_template, tc.show_menu_on_left_click)
        }
        None => (
            tauri::image::Image::from_bytes(include_bytes!("../icons/menuIconTray.png")).unwrap(),
            true,
            false,
        ),
    };

    let tray = TrayIconBuilder::new()
        .tooltip("OpenComputer")
        .icon(icon)
        .icon_as_template(icon_as_template)
        .show_menu_on_left_click(show_menu_on_left_click)
        .menu(&menu)
        .on_menu_event(|app_handle, event| {
            app_debug!(
                "tray",
                "menu",
                "Tray menu item clicked: {}",
                event.id().as_ref()
            );
            match event.id().as_ref() {
                "show_main" => {
                    show_main_window(app_handle);
                }
                "quick_chat" => {
                    crate::toggle_quickchat_window(app_handle);
                }
                "new_session" => {
                    show_main_window(app_handle);
                    let _ = app_handle.emit("new-session", ());
                }
                "open_settings" => {
                    show_main_window(app_handle);
                    let _ = app_handle.emit("open-settings", ());
                }
                "quit_app" => {
                    app_handle.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                app_debug!(
                    "tray",
                    "icon",
                    "Tray icon click: button={:?}, state={:?}",
                    button,
                    button_state
                );

                // Left click on tray icon → show main window.
                if button == MouseButton::Left && button_state == MouseButtonState::Up {
                    show_main_window(tray.app_handle());
                }
            }
        })
        .build(app)?;

    app_info!(
        "tray",
        "setup",
        "Tray initialized: id={}, show_menu_on_left_click={}, icon_as_template={}",
        tray.id().as_ref(),
        show_menu_on_left_click,
        icon_as_template
    );

    Ok(())
}
