# 01 · Getting Started

This chapter helps you install Hope Agent in a few minutes, complete the initial setup, start your first conversation, and understand the three run modes, how to update, and how to reach it remotely from other devices.

> For details on configuring models and API keys, see [02 · Models & Providers](02-models-and-providers.md); to use a fully offline local model, see the "One-click local model install" section in that chapter.

---

## 1.1 Installation

Complete installers for every platform are on [GitHub Releases](https://github.com/shiwenwen/hope-agent/releases). Below is the recommended approach per platform.

### macOS (most complete support)

**Option 1: Homebrew (recommended)**

```bash
brew tap shiwenwen/hope-agent
brew install --cask hope-agent
```

> Already installed `Hope Agent.app` manually? Add `--adopt` after `brew install` (adopt the existing app of the same version without re-downloading) or `--force` (force a re-download and overwrite).

**Option 2: install the DMG manually**

Download `Hope.Agent_*.dmg` from Releases and drag it into "Applications". Both Apple Silicon and Intel Macs have native builds (arm64 / x64); both Homebrew and the manual download automatically pick the right one for your hardware.

> **If launching shows "damaged" or "cannot verify the developer"**, run the following two lines in a terminal and reopen the app:
>
> ```bash
> sudo xattr -cr /Applications/Hope\ Agent.app
> sudo codesign --force --deep --sign - /Applications/Hope\ Agent.app
> ```
>
> This happens because the app uses an ad-hoc signature; it does not mean anything is wrong with the app itself.

### Windows (experimental)

**Option 1: Scoop (recommended)**

```powershell
scoop bucket add hope-agent https://github.com/shiwenwen/scoop-hope-agent
scoop install hope-agent
```

**Option 2: install manually**

Download `Hope.Agent_*-setup.exe` from Releases and double-click to install. Only an x64 build is currently provided.

> If launching reports a missing `MSVCP140_1.dll` / `VCRUNTIME140.dll` / `MSVCP140.dll`, install the [Microsoft Visual C++ 2015–2022 Redistributable (x64)](https://aka.ms/vs/17/release/vc_redist.x64.exe) and restart the app.
>
> The Windows build has not been thoroughly tested; feedback is welcome.

### Linux (experimental)

Native builds are provided for both amd64 (x86_64) and arm64 (aarch64), covering ordinary PCs, Raspberry Pi 4/5, Apple Silicon running Asahi Linux, and Graviton / Ampere cloud instances.

- **Arch / Manjaro**: `yay -S hope-agent-bin` (or paru / any AUR helper)
- **Debian / Ubuntu (apt)**:
  ```bash
  curl -fsSL https://shiwenwen.github.io/hope-agent-linux-repo/pubkey.gpg | \
    sudo gpg --dearmor -o /usr/share/keyrings/hope-agent.gpg
  echo "deb [signed-by=/usr/share/keyrings/hope-agent.gpg] https://shiwenwen.github.io/hope-agent-linux-repo/apt stable main" | \
    sudo tee /etc/apt/sources.list.d/hope-agent.list
  sudo apt update && sudo apt install hope-agent
  ```
- **Fedora / RHEL / CentOS (dnf / yum)**:
  ```bash
  sudo curl -fsSL https://shiwenwen.github.io/hope-agent-linux-repo/rpm/hope-agent.repo \
    -o /etc/yum.repos.d/hope-agent.repo
  sudo dnf install hope-agent
  ```
- **openSUSE**: `sudo zypper addrepo …/rpm/hope-agent.repo && sudo zypper install hope-agent`
- **Manual**: download the `.AppImage` (`chmod +x`, then run), `.deb` (`sudo dpkg -i`), or `.rpm` (`sudo rpm -i`); the package name carries an architecture suffix, so pick `_amd64` / `_arm64` for your machine.

---

## 1.2 First launch & configuration

The first time you open Hope Agent, a **setup wizard** appears. The main path is a single sentence — **pick a Provider template → enter an API key / sign in with Codex OAuth → start chatting**; every other step can be skipped or adjusted later in Settings.

The desktop wizard is actually 6 steps (the X in the top-right of each step exits; drafts are saved automatically, and next time you resume from the last step):

1. **Welcome** — choose the interface language (follows the system by default, 10+ languages supported) and theme (Automatic / Light / Dark); changes take effect instantly. At the bottom of the page there is also a collapsible "Connect to a remote server" entry (see [1.4](#14-access-from-your-phone-or-another-computer)).
2. **Model Provider (required)** — pick a service from the 40+ built-in templates (Anthropic, OpenAI, DeepSeek, Tongyi Qianwen, and more) and enter an API key; with Codex you can sign in to your account directly via OAuth, no manual key needed. This step also embeds the "One-click local model install" card. The skip button is highlighted in red as a reminder.
3. **Search service (optional)** — configure web search (DuckDuckGo / Tavily / Brave / Google, etc.); can be skipped.
4. **User profile (optional)** — enter your name, time zone, level of AI experience, and reply preferences to help the AI fit you better.
5. **Approval & security** — a single switch decides whether tool approval is enabled. On (default) = a confirmation dialog before dangerous operations; off = all tools are auto-approved (equivalent to full-auto mode).
6. **IM channels (optional)** — shows channel cards for Telegram / Feishu / Slack and others; you can add an account right here, or configure it later in Settings. Clicking "Finish" ends the wizard and takes you into the conversation.

> - Don't want to use a cloud model? **One-click local model install** is right there in step 2 — no account, API key, or terminal required. See [02 · Models & Providers · Local models](02-models-and-providers.md).
> - Want to run through the wizard again? Run `hope-agent server setup` on the command line (this replays the wizard by default; adding `--reset` first clears the existing "onboarding complete" state — Providers and user settings you have already configured are not deleted).

All data — configuration, sessions, memory, and so on — is stored in the `~/.hope-agent/` directory on your own machine (use the `HA_DATA_DIR` environment variable to change the location).

---

## 1.3 Three run modes

Hope Agent's three run modes **share the same core logic**; configuration, sessions, and memory all live in `~/.hope-agent/`, and you can switch between them at any time.

| Mode | How to start | Best for |
| --- | --- | --- |
| **Desktop GUI** | Double-click the app icon, or run `hope-agent` / `open -a "Hope Agent"` in a terminal | The most complete entry point: full native GUI, with an embedded HTTP/WS service — while you use the desktop, it can also serve connections to others |
| **Server + Web GUI** | `hope-agent server start`; `hope-agent server install` registers it as a system service that starts on boot | A windowless daemon, online 24/7 so IM channels / scheduled tasks never drop; the full web version is embedded in the service — open it in a browser and it just works, and phones / tablets / any computer can connect directly |
| **ACP (IDE integration)** | `hope-agent acp` | Lets an ACP-compatible editor call Hope Agent as its AI backend |

**Launch entry points per platform:**

- **Desktop GUI**: click the icon in Launchpad / Start menu / applications menu, or run `hope-agent` in a terminal.
- **Browser Web GUI**: with the desktop app open, visit <http://127.0.0.1:8420>; you can also run only `hope-agent server start` to start the service without opening the desktop window.
- **ACP**: `hope-agent acp`.

### Common Server-mode subcommands

```bash
hope-agent server start        # start the HTTP/WS service in the foreground
hope-agent server install      # register a system service (macOS launchd / Linux systemd), auto-start on boot
hope-agent server status       # check run status
hope-agent server stop         # stop the service
hope-agent server uninstall    # uninstall the system service
hope-agent server setup        # re-run the setup wizard (replays by default; --reset clears the "onboarding complete" state first, keeps Providers)
```

---

## 1.4 Access from your phone or another computer

To reach a Hope Agent running on your main device from a phone, tablet, or another computer:

1. Open "**Settings → Server**" and **set an API key** (used for authentication — be sure to set it).
2. Change the listen address from `127.0.0.1:8420` to `0.0.0.0:8420` (allow LAN access).
3. After restarting, open `http://<IP of the device running Hope Agent>:8420` in a browser on the other device and enter the API key.

> ⚠️ **Security reminder**: do not expose the port to your LAN or the public internet without authentication (no API key set). When you need public access, put an HTTPS reverse proxy in front of it — see the [Docker deployment guide](../../deployment/docker.md).

---

## 1.5 Self-hosting (Docker)

To run Hope Agent on a home NAS, a VPS, or a homelab and access the web version from a browser:

```bash
docker run -d \
  --name hope-agent \
  -p 127.0.0.1:8420:8420 \
  -v hope-data:/data \
  ghcr.io/shiwenwen/hope-agent:latest
```

Once the container is up, open <http://127.0.0.1:8420> in a browser and configure your Provider API key through the setup wizard. The image covers `linux/amd64` + `linux/arm64` (including Apple Silicon / Raspberry Pi) and is built automatically with every release.

For docker compose, pairing with a local Ollama LLM, exposing to the LAN, reverse proxy and TLS, and the upgrade process, see the full [Docker deployment guide](../../deployment/docker.md). You can also bring up a local LLM sidecar in one command:

```bash
docker compose --profile with-ollama up -d    # start hope-agent + a local Ollama model
```

---

## 1.6 Updating to a new version

The desktop app has a built-in auto-updater based on GitHub Releases. There are three ways to update — pick whichever you like:

1. **Check in-app**: open "**Settings → About**", click Check for updates, and install a new version in one click when found.
2. **Just say so in a conversation**: say "upgrade" or "check for updates" directly in a conversation, and the AI will use the `app_update` tool to check for you, then install after you confirm.
3. **Wait for the app to prompt**: the app notifies you when a new version is available.

> - Versions installed via Homebrew / AUR / Scoop **also use the app's built-in updater**; the version number seen from the package manager's perspective stays at the value from initial install, which does not affect functionality and is expected.
> - The update process first verifies the signature, then runs a cold-start self-check after downloading; on failure it rolls back automatically, doing its best to ensure you never update into a version that won't open.
> - The auto-update behavior (whether to pre-download in the background, whether manual confirmation is required before installing) can be adjusted under "Settings → About / Auto-update". The desktop version **will never automatically restart and replace itself without your confirmation**.

---

## Next steps

- Got your model configured? → [02 · Models & Providers](02-models-and-providers.md)
- Ready to start a real conversation? → [03 · Chat & Sessions](03-chat-and-sessions.md)
- Want to know where all the settings are? → [13 · Settings & Security](13-settings-and-security.md)
