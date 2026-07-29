//! Self-update for `hope-agent` across desktop / `hope-agent server` / CLI.
//!
//! Three upgrade routes, picked by the running formfactor + install source:
//!
//! 1. **Desktop bundle** (DMG / MSI / NSIS / AppImage) — keep the existing
//!    `tauri-plugin-updater` path. The tool layer routes here when
//!    [`ha_core::is_desktop`] is true; this module exposes only the manifest
//!    fetch / version compare helpers in that case.
//! 2. **Package manager** (`brew` / `scoop` / `apt` / `dnf` / `pacman`) —
//!    detect the install source ([`source_detector`]) and run the matching
//!    upgrade command via [`package_manager`], then restart the service via
//!    [`service_control`].
//! 3. **Self-contained binary swap** — download the bare-binary archive
//!    keyed in the manifest, verify the Minisign signature, atomically
//!    replace the executable, and restart the service. Always available as
//!    a fallback when path 2 fails or the install source is `manual`.
//!
//! All long-running work flows through an `async_jobs` job so the model
//! can `job_status` it and the user can interrupt cleanly. Progress is
//! mirrored onto the `app_update:progress` EventBus topic for the UI.
//!
//! Hard rules:
//!
//! - Every downloaded artifact MUST pass [`signature::verify_bytes`]
//!   before it's allowed to overwrite anything. No exceptions, no
//!   "trust the SHA from the manifest" shortcut — the Minisign pubkey
//!   is the single root of trust shared with `tauri-plugin-updater`.
//! - Binary swap goes through [`ha_core::platform::atomic_replace_binary`].
//!   Plain `fs::write` over the live image is forbidden — even on Unix
//!   where it would "work" it leaves the file half-written if we crash.
//! - Downloads are byte-capped at [`download::MAX_DOWNLOAD_BYTES`] so a
//!   tampered manifest can't fill `~/.hope-agent/updater/staging/`.

// `app_info!` 系宏由 ha-base 导出（宏体经 `$crate::` 解析回 ha-base，与
// 调用方无关）——与 ha-core 同一接法。
#[macro_use]
extern crate ha_base;

pub mod auto_check;
pub mod backup;
pub mod config;
pub mod download;
pub mod keys;
pub mod manifest;
pub mod package_manager;
pub mod self_contained;
pub mod service_control;
pub mod signature;
pub mod source_detector;
pub mod staging;
mod tool;

use std::sync::{Arc, OnceLock};

use anyhow::Result;
use async_trait::async_trait;

/// 装配入口：把 `app_update` 工具分发条目挂进 ha-core 注册表，并把 headless
/// 自动更新循环登记为 init 启动任务。**每个会调 `ha_core::init_runtime` 的
/// 二进制（`hope-agent` 桌面壳 / `hope-agent-server` / ha-eval runner）必须
/// 在 init 之前调用一次**——init 尾部冻结注册表，晚了直接 panic（fail-loud，
/// 静默漏接＝`app_update` 有 schema 无 handler，模型调用报 Unknown tool）。
/// 幂等，重复调用 no-op。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        fn app_update_handler<'a>(
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(tool::tool_app_update(args, ctx))
        }
        ha_core::tools::registry::register_external_tools(vec![
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_APP_UPDATE,
                aliases: &[],
                handler: app_update_handler,
            },
        ])
        .expect(
            "ha_updater::wire() must run before ha_core::init_runtime freezes the tool registry",
        );
        // 桌面 no-op 判定在 spawn_auto_update_loop 内部（`is_desktop`）；
        // Primary 门由消费点保证——startup task 在 `start_background_tasks`
        // 的 primary-gated 块执行（原 spawn_auto_update_loop 调用位），故
        // 仅 desktop-GUI / server 形态且 Primary 进程才真正运行 loop，与
        // 迁移前逐位一致（acp / mcp / eval 注册了也不消费）。
        ha_core::app_init::register_startup_task(
            ha_core::app_init::StartupStage::PrimaryOnly,
            || {
                auto_check::spawn_auto_update_loop();
            },
        )
        .expect("ha_updater::wire() must run before start_background_tasks consumes startup tasks");
    });
}

pub use config::AutoUpdateConfig;
use serde::Serialize;

use source_detector::InstallSource;

/// Hook the desktop shell uses to register `tauri-plugin-updater` so the
/// Tauri-bundled formfactor can self-update without ha-core gaining a Tauri
/// dependency. `src-tauri/src/commands/update_bridge.rs` calls
/// [`set_updater_bridge`] in `setup.rs`; the tool layer probes
/// [`get_updater_bridge`] when [`recommend_path`] returns `Tauri`.
#[async_trait]
pub trait UpdaterBridge: Send + Sync {
    /// Run the bundled-installer upgrade: download → verify (Minisign) →
    /// install → restart. Implementations block until the new image is
    /// staged; the OS bundle replacement may continue after the process
    /// dies. The caller (`app_update install`) reports `started` to the
    /// model and waits on the EventBus for completion frames.
    async fn install_and_restart(&self, job_id: &str) -> Result<String>;
}

static UPDATER_BRIDGE: OnceLock<Arc<dyn UpdaterBridge>> = OnceLock::new();

pub fn set_updater_bridge(bridge: Arc<dyn UpdaterBridge>) {
    // OnceLock::set fails if already set; that's fine — both desktop and
    // tests should only register a bridge once per process.
    let _ = UPDATER_BRIDGE.set(bridge);
}

pub fn get_updater_bridge() -> Option<Arc<dyn UpdaterBridge>> {
    UPDATER_BRIDGE.get().cloned()
}

/// Outcome of `app_update check` — readonly snapshot the model uses to
/// describe the upgrade situation to the user and decide whether to ask
/// for install permission.
#[derive(Debug, Clone, Serialize)]
pub struct CheckOutcome {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub platform_target: &'static str,
    pub install_source: InstallSource,
    pub recommended_path: RecommendedPath,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    /// `true` iff the manifest carries a `bare_binary` entry for this
    /// platform. When the desktop path isn't applicable and this is
    /// `false`, the only remaining option is the package-manager path
    /// (or asking the user for a manual download).
    pub bare_binary_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedPath {
    /// Desktop is in the foreground — route to `tauri-plugin-updater`.
    Tauri,
    /// Package manager owns this install — run the matching upgrade
    /// command, then restart the service.
    PackageManager,
    /// Download the bare-binary archive, verify, swap, restart.
    SelfContained,
    /// No actionable path — surface the gap to the user (e.g. headless
    /// install via a channel the bare-binary release doesn't cover yet).
    ManualPrompt,
}

pub async fn check_update() -> Result<CheckOutcome> {
    let (outcome, _) = check_update_full().await?;
    Ok(outcome)
}

/// Same as [`check_update`] but also returns the fetched [`manifest::Manifest`]
/// so callers chaining into the install path don't pay a second HTTP fetch.
pub async fn check_update_full() -> Result<(CheckOutcome, manifest::Manifest)> {
    let manifest = manifest::fetch_manifest().await?;
    let outcome = build_check_outcome(&manifest);
    Ok((outcome, manifest))
}

fn build_check_outcome(manifest: &manifest::Manifest) -> CheckOutcome {
    let current_version = ha_core::app_init::app_version().to_string();
    let latest_version = manifest.version.clone();
    let has_update = manifest::is_newer(&latest_version, &current_version);
    let platform_target = manifest::current_platform_key();
    let install_source = source_detector::detect_install_source();
    let bare_binary_available = manifest::select_bare_binary(manifest, platform_target).is_some();
    let recommended_path = recommend_path(&install_source, bare_binary_available);
    CheckOutcome {
        current_version,
        latest_version,
        has_update,
        platform_target,
        install_source,
        recommended_path,
        notes: manifest.notes.clone(),
        pub_date: manifest.pub_date.clone(),
        bare_binary_available,
    }
}

/// Decide which upgrade route fits the running formfactor + install source.
///
/// Routing rules (first match wins):
/// 1. Desktop in the foreground → `Tauri` (signed installer flow already
///    wired up in `tauri-plugin-updater`).
/// 2. Install source is one of the package managers → `PackageManager`.
/// 3. Headless install with a bare-binary archive available → `SelfContained`.
/// 4. Nothing applies → `ManualPrompt` (the tool layer prompts the user via
///    `ask_user_question` to download manually).
pub fn recommend_path(source: &InstallSource, bare_binary_available: bool) -> RecommendedPath {
    if ha_core::app_init::is_desktop() && matches!(source, InstallSource::TauriBundle) {
        return RecommendedPath::Tauri;
    }
    match source {
        InstallSource::Brew { .. }
        | InstallSource::Scoop
        | InstallSource::Aur
        | InstallSource::Apt
        | InstallSource::Dnf => RecommendedPath::PackageManager,
        // Container deployment: binary swap inside the container is wiped
        // on the next `docker pull`. Always prompt the user to recreate the
        // container instead.
        InstallSource::Docker => RecommendedPath::ManualPrompt,
        InstallSource::TauriBundle => {
            // Headless `hope-agent server` running off the desktop bundle
            // (rare but happens when users start the daemon from `/Applications`
            // without launching the GUI). The bundle has no package manager
            // we can drive from here, so fall through to self-contained if
            // it's available, otherwise prompt.
            if bare_binary_available {
                RecommendedPath::SelfContained
            } else {
                RecommendedPath::ManualPrompt
            }
        }
        InstallSource::Manual => {
            if bare_binary_available {
                RecommendedPath::SelfContained
            } else {
                RecommendedPath::ManualPrompt
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn recommend_brew_routes_to_package_manager() {
        let s = InstallSource::Brew {
            prefix: PathBuf::from("/opt/homebrew"),
        };
        assert_eq!(recommend_path(&s, false), RecommendedPath::PackageManager);
    }

    #[test]
    fn recommend_manual_with_bare_binary_uses_self_contained() {
        assert_eq!(
            recommend_path(&InstallSource::Manual, true),
            RecommendedPath::SelfContained
        );
    }

    #[test]
    fn recommend_manual_without_bare_binary_falls_back_to_prompt() {
        assert_eq!(
            recommend_path(&InstallSource::Manual, false),
            RecommendedPath::ManualPrompt
        );
    }

    #[test]
    fn recommend_docker_always_routes_to_manual_prompt() {
        // Bare-binary availability is irrelevant — container binary swap
        // would be wiped by the next `docker pull`, so we always prompt.
        assert_eq!(
            recommend_path(&InstallSource::Docker, true),
            RecommendedPath::ManualPrompt
        );
        assert_eq!(
            recommend_path(&InstallSource::Docker, false),
            RecommendedPath::ManualPrompt
        );
    }
}
