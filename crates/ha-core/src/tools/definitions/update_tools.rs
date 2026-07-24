use serde_json::json;

use super::super::TOOL_APP_UPDATE;
use super::types::{CoreSubclass, ToolDefinition, ToolTier};

pub fn get_app_update_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_APP_UPDATE.into(),
        description: "Check for and install Hope Agent updates across desktop / `hope-agent server` / CLI modes.\n\n\
Actions:\n\
- `check` — read-only. Returns `{current_version, latest_version, has_update, recommended_path, install_source, notes, pub_date, bare_binary_available}`. Use this first to decide whether to suggest an upgrade.\n\
- `install` — perform the upgrade. Asks the user for confirmation via `ask_user_question` before doing anything (cannot be bypassed). On confirm, downloads the new binary, verifies its Minisign signature against the same pubkey `tauri-plugin-updater` uses, atomically swaps the executable, and restarts the user service (`launchctl` / `systemctl --user` / Windows Task Scheduler). Long-running — pass `run_in_background: true`, rely on the completion notification, and use `job_status` only for an occasional snapshot if the user asks.\n\
- `status` — given a `job_id` from a prior `install`, returns the current phase (`downloading | verifying | staging | swapping | restarting | done | failed`) and percentage. Read-only.\n\
- `rollback` — restore the previous binary from `~/.hope-agent/updater/backup/`. Asks the user before swapping. Use when an install completed but the new version misbehaves.\n\n\
Path routing (`install` picks automatically unless `prefer_path` is set):\n\
- Desktop GUI in foreground → routes through `tauri-plugin-updater` (signed installer). Frontend handles the install UI.\n\
- Homebrew / Scoop / AUR / apt / dnf install → runs the matching package-manager upgrade command, then restarts the service.\n\
- Manual install (single binary drop, dev build) → downloads the bare-binary archive, verifies signature, atomically replaces the executable.\n\n\
When any path fails the tool emits a structured error via `ask_user_question` so the user can choose between retry / switch path / cancel — do not try to recover yourself by re-invoking with different args."
            .into(),
        // Meta tier: always-eager schema (high-value, low-frequency
        // capability that nudges the model toward suggesting upgrades).
        // Not `internal`: install / rollback have side effects and the
        // tool gates them on its own `ask_user_question` confirmation,
        // not the generic permission engine's edit-tool layer.
        tier: ToolTier::Core {
            subclass: CoreSubclass::Meta,
        },
        internal: false,
        concurrent_safe: false,
        // `install` is the long-running action — `check` / `status` /
        // `rollback` return in well under a second. GenericJob
        // lets the model opt into `run_in_background: true` for install.
        background_policy: crate::tools::definitions::BackgroundPolicy::GenericJob,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["check", "install", "status", "rollback"],
                    "description": "Which sub-operation to run."
                },
                "target_version": {
                    "type": "string",
                    "description": "Pin install to a specific version (e.g. \"0.2.1\"). Defaults to the latest version in the release manifest. Ignored for non-install actions."
                },
                "prefer_path": {
                    "type": "string",
                    "enum": ["auto", "package_manager", "self_contained"],
                    "description": "Override the auto-selected upgrade route. `auto` (default) uses the install-source detector. `package_manager` forces brew/scoop/apt/dnf even on a manual install (will fail if the binary isn't actually owned by one). `self_contained` forces the bare-binary swap. Used by the recovery prompt after a path-specific failure."
                },
                "job_id": {
                    "type": "string",
                    "description": "Required for `status`: the job id returned by a prior `install` call."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}
