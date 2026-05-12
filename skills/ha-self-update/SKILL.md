---
name: ha-self-update
description: "Check for and install Hope Agent updates through conversation. Use whenever the user asks about upgrades, new versions, release notes, or reports a bug that might already be fixed upstream — phrases like 'upgrade Hope Agent', 'update hope agent', 'check for new version', '升级一下', '有新版本吗', '帮我升级', 'is there a newer build', 'check release notes', 'install the latest'. Also use proactively when an `app_update(action=\"check\")` result shows `has_update: true` and the user hasn't been told yet. Covers all three formfactors: desktop GUI bundle (DMG/MSI/AppImage), `hope-agent server` daemon installed via Homebrew/Scoop/AUR/apt/dnf, and headless single-binary deployments. The upgrade is always user-confirmed via `ask_user_question` — never silent."
always: false
aliases:
  - self-update
  - upgrade
---

# Hope Agent Self-Update

Hope Agent ships a single binary (`hope-agent`) that dispatches into three modes by subcommand: desktop GUI, `hope-agent server`, and `hope-agent acp`. All three share the same release artifacts under [github.com/shiwenwen/hope-agent/releases](https://github.com/shiwenwen/hope-agent/releases) and the same Minisign-signed update manifest. The `app_update` tool is the single entry point for self-upgrade; this skill is the methodology for using it well.

## When to suggest an upgrade

Trigger paths (any one is enough):

- User asks ("升级一下" / "is there an update" / "show release notes" / "the latest version").
- User reports a bug. Run `app_update(action="check")` first — if `has_update: true`, mention "there's a newer version `latest_version`; let me check its notes for `<bug-description>` before we dig in" and read the notes. If the notes mention the bug, suggest upgrading first.
- A startup snapshot shows the agent hasn't upgraded in a long while AND `has_update` is true. Don't nag — bring it up once per conversation at most.

Do NOT trigger if the user is mid-task on something that would be disrupted (active chat turn from a subagent, in-flight cron run, IM media uploading). Wait or finish first.

## Workflow

### 1. Check — never skip

```
app_update(action="check")
```

Returns:

```json
{
  "current_version": "0.1.1",
  "latest_version": "0.2.0",
  "has_update": true,
  "install_source": { "kind": "brew", "prefix": "/opt/homebrew" },
  "recommended_path": "package_manager",
  "platform_target": "darwin-aarch64",
  "notes": "fix: …",
  "pub_date": "2026-05-12T10:00:00Z",
  "bare_binary_available": true
}
```

Read the result aloud (versions + 1-2 lines from notes). If `has_update == false` and the user asked to upgrade anyway (e.g. for a force-reinstall), surface that and ask if they really want to pin a `target_version`.

### 2. Pre-flight before recommending install

Quick checks before calling `app_update(action="install")` — these aren't blocking, but mention them:

- Use `exec` to peek at active workload: `launchctl list | grep hopeagent` / `systemctl --user is-active hope-agent.service`, plus `sessions_list({ limit: 5 })` to see if any session is mid-turn.
- If a cron job is scheduled in the next few minutes, suggest waiting.
- On macOS: warn if `Hope Agent.app` is open and the user is mid-task — restart will kill in-flight turns.

### 3. Install — pass `run_in_background: true`

```
app_update(action="install", run_in_background: true)
```

`run_in_background: true` is recommended for `install` (download + verify + swap takes 10s-2min depending on connection). The tool returns `{ job_id, status: "started" }` immediately. The user sees a confirmation dialog (Yes/No) — that dialog cannot be bypassed.

If they decline, the tool returns `cancelled_by_user`. Don't try again automatically — wait for the user to ask.

### 4. Track progress

Two ways to follow along:

- `app_update(action="status", job_id="...")` — polls the in-memory phase tracker.
- Frontend subscribes to EventBus topic `app_update:progress` — the UI renders the progress bar automatically; the tool also emits `app_update:completed` when the job finishes.

Phases (in order): `starting → running → downloading → verifying → staging → backing → swapping → restarting → done`. Failure transitions straight to `failed` with an `error` field.

### 5. Verify after install

The service restarts itself on success, but the binary swap is only visible to processes started AFTER the swap — the model conversation is still running the old image. Tell the user:

> Upgrade succeeded. The server service has been restarted on the new image. Your desktop GUI is still running the old version — quit and reopen to load v0.2.0.

Then `exec` to confirm:

```bash
hope-agent --version    # should print the new version
curl -s http://127.0.0.1:8420/api/health    # if server is configured
```

## Path routing

`recommended_path` in the check output is the auto-selected route. The user can override via `prefer_path` on `install`:

| Path              | When                                          | What happens                                                                 |
| ----------------- | --------------------------------------------- | ---------------------------------------------------------------------------- |
| `tauri`           | Desktop GUI in foreground, bridge registered  | `tauri-plugin-updater` downloads + verifies + installs the signed bundle.    |
| `package_manager` | brew / scoop / apt / dnf / AUR install        | Runs `brew upgrade --cask hope-agent` (etc.), then restarts the service.     |
| `self_contained`  | Manual install, or above paths unavailable    | Downloads bare-binary tar.gz, verifies Minisign sig, atomic-swap, restart.   |
| `manual_prompt`   | Cannot pick automatically                     | Tool prompts the user via `ask_user_question` to choose recovery.            |

`prefer_path: "self_contained"` is the right fallback when the package-manager path fails (stale tap, sudo refused, etc.). `prefer_path: "package_manager"` is rarely needed — only when the user wants brew/apt to record the new version.

## When things fail

The tool surfaces failures via `app_update(action="status")`'s `error` field. Common cases:

| Error contains                            | Recover with                                                                |
| ----------------------------------------- | --------------------------------------------------------------------------- |
| `minisign verify failed`                  | Re-run install (download may have been truncated). If it persists twice, treat as a release-signing problem — DO NOT bypass; tell the user to report it. |
| `HTTP 4xx/5xx from <url>`                 | Network / GitHub Releases hiccup. Wait a minute, retry. Suggest a manual download if persistent. |
| `manifest has no bare_binary entry for…`  | Switch to `prefer_path: "package_manager"`. If that also doesn't apply, ask the user to download from the release page manually. |
| `package manager upgrade failed`          | Show the stderr to the user. Often: `sudo` denied, stale apt cache (`sudo apt update` first), brew tap not synced (`brew update` first). |
| `service restart failed`                  | The new binary is in place but the service didn't come back. Walk the user through `launchctl kickstart -k gui/$UID/ai.hopeagent.server` / `systemctl --user restart hope-agent.service` manually. |
| `atomic swap … failed`                    | Suggest `app_update(action="rollback")` to restore the previous binary, then investigate before retrying. |

## Rollback

If a successful install behaves badly, the previous binary is in `~/.hope-agent/updater/backup/<old-version>/hope-agent`. Restore via:

```
app_update(action="rollback")
```

Same Yes/No confirmation as install. The tool restores the most recent backup, restarts the service. Only the most recent 2 backups are retained.

## What this skill cannot do

- **Skip the user confirmation.** `install` and `rollback` always pop the Yes/No dialog. Trying to bypass it would defeat the whole point of HIGH-risk gating.
- **Change the update endpoint.** `latest.json` lives at the URL hardcoded in `ha_core::updater::manifest::UPDATE_MANIFEST_URL` and pinned by both the desktop bundle and `ha-core/updater/keys.rs` pubkey.
- **Switch release channels.** Only the stable channel is supported today. Beta/nightly is on the roadmap but not yet wired up.
- **Restart the desktop app for the user.** After the binary swap completes, the user has to quit and relaunch the desktop GUI themselves — this is intentional so they don't lose in-flight chat state.
