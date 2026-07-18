<p align="center">
  <img src="assets/alpha-logo.png" alt="Hope Agent" width="200">
</p>

<h1 align="center">Hope Agent</h1>

<p align="center">
  <strong>A desktop AI assistant that hands off across your devices and gets to know you better the more you use it — also runs headless, on a NAS or in the cloud.</strong><br/>
  Remembers you · Grows over time · Pursues goals autonomously · Orchestrates work dynamically · Reachable from every chat app you use
</p>

<p align="center">
  <a href="https://github.com/shiwenwen/hope-agent/actions/workflows/rust.yml"><img src="https://img.shields.io/github/actions/workflow/status/shiwenwen/hope-agent/rust.yml?branch=main&style=flat-square&logo=githubactions&logoColor=white&label=CI" alt="CI status"></a>
  <a href="https://github.com/shiwenwen/hope-agent/releases"><img src="https://img.shields.io/badge/macOS-000000?style=flat-square&logo=apple&logoColor=white" alt="macOS"></a>
  <a href="https://github.com/shiwenwen/hope-agent/releases"><img src="https://img.shields.io/badge/Linux-experimental-FFA500?style=flat-square&logo=linux&logoColor=black" alt="Linux (experimental)"></a>
  <a href="https://github.com/shiwenwen/hope-agent/releases"><img src="https://img.shields.io/badge/Windows-experimental-FFA500?style=flat-square&logo=data:image/svg%2Bxml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI+PHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDMuNDQ5TDkuNzUgMi4xdjkuNDUxSDBtMTAuOTQ5LTkuNjAyTDI0IDB2MTEuNEgxMC45NDlNMCAxMi42aDkuNzV2OS40NTFMMCAyMC42OTlNMTAuOTQ5IDEyLjZIMjRWMjRsLTEyLjktMS44MDEiLz48L3N2Zz4=" alt="Windows (experimental)"></a>
  <a href="#run-modes"><img src="https://img.shields.io/badge/Web%20GUI-browser-4F46E5?style=flat-square&logo=googlechrome&logoColor=white" alt="Web GUI"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-edition%202021-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="https://tauri.app/"><img src="https://img.shields.io/badge/Tauri-2-24C8DB?style=flat-square&logo=tauri&logoColor=white" alt="Tauri"></a>
  <a href="https://react.dev/"><img src="https://img.shields.io/badge/React-19-61DAFB?style=flat-square&logo=react&logoColor=black" alt="React"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=flat-square" alt="License: MIT"></a>
</p>

<p align="center">
  <a href="./README.md">简体中文</a> · <strong>English</strong>
</p>

---

**Hope Agent** is a local-first, desktop-first personal AI agent that can also run continuously as a service. It opens like mature desktop software while offering real agentic capability: understanding long-term context, using tools to complete work, and reliably pursuing goals even after you step away.

**Hope Agent did not begin after “desktop AI agents” became a popular category.** We started building it when desktop agents were still uncommon and Codex itself was in an earlier form. We have always focused on the product itself, not on manufacturing attention. As more products have moved in the same direction, they have reinforced our original conviction: **AI assistants will move beyond the chat box and become personal software people can trust with long-running work.**

## Contents

- [Core Capabilities](#core-capabilities)
- [Capability Overview](#capability-overview)
- [Quick Start](#quick-start)
  - [Install Locally](#install-locally)
  - [Self-hosting (Docker)](#self-hosting-docker)
  - [For developers](#for-developers)
- [Run Modes](#run-modes)
- [Ecosystem](#ecosystem)
- [Project Structure](#project-structure)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [Community](#community)
- [Acknowledgements](#acknowledgements)
- [Star History](#star-history)
- [License](#license)

## Core Capabilities

<table>
<tr><td width="220"><b>🖥️ A desktop agent that just works</b></td><td>A native GUI, a broad set of provider templates and preset models, and one-click local model setup. Paste an API key or sign in—no runtime setup, CLI learning curve, or config sprawl required.</td></tr>
<tr><td><b>🧭 Completes work, not just replies</b></td><td>Goal defines the outcome, Workflow dynamically organizes execution, and Loop decides when to continue. Complex work can progress in the background while remaining visible, pausable, resumable, and adjustable.</td></tr>
<tr><td><b>🧠 Long-term memory and knowledge</b></td><td>Cross-session memory, on-demand recall, offline consolidation, user profiles, and reusable skills compound over time. Knowledge Space lets you and the AI read and write real Markdown together as a traceable second brain.</td></tr>
<tr><td><b>🎨 From an idea to a deliverable design</b></td><td>Design Space turns a prompt or reference image into 10+ artifact types, including websites, mobile prototypes, and presentations, with visual editing, version history, multi-format export, and handoff into a real codebase.</td></tr>
<tr><td><b>🛠️ Acts in the real environment</b></td><td>With permission and approval, Hope Agent can control the computer and browser, run commands, edit files, call MCP tools, connect to workspaces, and coordinate multiple agents.</td></tr>
<tr><td><b>🌐 Handoff across devices, always available</b></td><td>Desktop / Server / Web / ACP share the same core. The same session, memory, and task state can continue across desktop, browser, and popular IM channels.</td></tr>
<tr><td><b>🛡️ Local-first, controlled, dependable</b></td><td>Data stays local by default and model requests go directly to providers. Tool approvals, Docker sandboxing, configuration rollback, crash recovery, and keepalive layers protect long-running operation.</td></tr>
</table>

## Capability Overview

### 🎨 Design & knowledge

<table>
<tr><td width="220"><b>🎨 Design Space</b></td><td>Generate websites, mobile prototypes, presentations, dashboards, posters, documents, email, images, motion, audio, and interactive components from a prompt or reference image. Generation streams into a live preview; each project includes AI chat, element-level editing, annotations, undo / redo, device previews, version history, an artifact library, and HTML / PNG / PDF / PPTX / MP4 / ZIP export.</td></tr>
<tr><td><b>🧩 Design system to code</b></td><td>Extract a brand system from screenshots, URLs, Figma, or an existing repository, reuse it across artifacts, and export multi-platform design tokens or a code handoff package. Bind a design project to a real repository, send an artifact to the main conversation for implementation, and surface later code changes back in the design.</td></tr>
<tr><td><b>🧠 Knowledge Space · second brain</b></td><td>You and the AI work on the same real Markdown notes, with source archiving, full-text and vector retrieval, backlinks, graph view, atomic notes, and reviewable AI organization proposals. Existing Obsidian vaults can be attached, external changes sync live, and source Evidence remains traceable.</td></tr>
</table>

### 🧭 Long-running work & autonomy

<table>
<tr><td width="220"><b>🎯 Goal · keep pursuing the outcome</b></td><td>Give Hope Agent an outcome and completion criteria, and it keeps decomposing, executing, checking, and advancing. Goals support budgets, progress, pause, and resume; completion requires a conservative audit with result evidence.</td></tr>
<tr><td><b>🧩 Workflow · dynamic orchestration</b></td><td>The model organizes phases, conditions, parallel work, multiple agents, tools, diffs, reviews, and verification when the task benefits from it. Every run has a durable trace with pause, resume, cancel, and conservative restart recovery.</td></tr>
<tr><td><b>🔁 Loop · continue on time or events</b></td><td>Continue on fixed intervals, conditions, internal events, or model-selected wakeups. Each iteration can resume the current conversation or launch a Goal-bound Workflow, with budgets, backoff, and no-progress protection.</td></tr>
<tr><td><b>📋 Plan, Task, and background work</b></td><td>Complex work can start with an editable plan while Tasks expose live progress. Long-running tools and sub-agents run in the background, and milestones return progressively to the main conversation without blocking the chat.</td></tr>
</table>

> Mental model: **Goal** defines the outcome, **Workflow** performs one concrete execution, **Loop** decides when to advance again, **Task** exposes current progress, and **Mode** controls execution autonomy. They compose cleanly but can also be used independently.

### 🧠 Memory & growth

<table>
<tr><td width="220"><b>🧠 Persistent cross-session memory</b></td><td>Memory is organized across Global, Project, and Agent scopes. A compact Core stays stable in context while detailed content returns on demand through full-text and vector retrieval instead of being resent every turn.</td></tr>
<tr><td><b>🔍 Recall, consolidation, and reflection</b></td><td>The model can recall memory when a task needs it, and users can enable Fast or Deep Recall. Idle-time consolidation produces a Dream Diary and can distill reviewable communication preferences, work habits, and long-term patterns from history.</td></tr>
<tr><td><b>🛠 Skills that grow</b></td><td>Complex work can become a draft skill for your review and later reuse. Skills support conditional activation, sub-agent execution, tool allowlists, and the <a href="https://agentskills.io">agentskills.io</a> standard.</td></tr>
<tr><td><b>💾 Long conversations and incognito mode</b></td><td>Progressive context compaction preserves key facts and tool-call relationships across long conversations. Incognito sessions disable long-term memory, cross-session awareness, and persistence paths, then remove the session data when the conversation ends.</td></tr>
</table>

### 🛠 Tools & connections

<table>
<tr><td width="220"><b>🖱️ Computer and browser control</b></td><td>On macOS, granted permissions let Hope Agent observe and operate the desktop, windows, menus, keyboard, and pointer. The controllable browser includes a live mirror so you can see the pages the Agent is visiting and manipulating. Side effects share one approval flow.</td></tr>
<tr><td><b>👥 Multiple agents and natural-language scheduling</b></td><td>Preset teams or dynamic sub-agents work in parallel and summarize results back to the main conversation. Natural-language schedules can run recurring work and deliver results to a chosen IM channel.</td></tr>
<tr><td><b>📁 Project containers</b></td><td>Keep related sessions, project instructions, memory, and shared files together. Uploaded files are extracted automatically and exposed as a directory, inline context, or on-demand reads according to size.</td></tr>
<tr><td><b>🔌 MCP and Hooks</b></td><td>The built-in MCP client covers major transports and OAuth 2.1. Hooks attach command / HTTP / MCP / prompt / agent handlers to 20+ lifecycle events, with layered configuration and hot reload.</td></tr>
<tr><td><b>🔧 Toolbox and workspaces</b></td><td>Built-ins include AI image generation, web search, bash, file operations, Canvas, URL preview, and self-diagnosis. Deep Feishu / Lark integration adds 40+ tools across documents, bitable, drive, wiki, approvals, calendar, contacts, and recruiting.</td></tr>
<tr><td><b>📊 Dashboard + Recap</b></td><td>Track cost, tokens, activity, health, Plans, and long-running work in one place. Recap reviews a period of conversation history, produces a multi-section report, and exports standalone HTML.</td></tr>
</table>

### 🌐 Desktop, service & cross-device

<table>
<tr><td width="220"><b>🖥️ Native GUI and model setup</b></td><td>macOS provides the complete desktop experience, while Linux and Windows support is currently experimental. The interface is multilingual, with broad provider coverage, a large preset model catalog, and automatic rotation across multiple API keys per provider.</td></tr>
<tr><td><b>🦙 One-click local models</b></td><td>No account, API key, or terminal required. Pick a model that fits your hardware and Hope Agent handles Ollama installation, model download, provider registration, and switching; local embedding models use the same flow.</td></tr>
<tr><td><b>🤝 IM channels and session handoff</b></td><td>Connect Telegram, Discord, Slack, Feishu, and other popular IM channels. Images, voice, and files become multimodal context; sessions hand off between desktop, browser, and IM, and active responses can live-mirror into chat apps.</td></tr>
<tr><td><b>🌐 Standalone service and multiple run modes</b></td><td><code>hope-agent server</code> stays online on a NAS, home server, or cloud VM with a full embedded Web GUI. <code>hope-agent acp</code> serves as an Agent backend for IDEs. Every entry point shares the same core, sessions, memory, and configuration.</td></tr>
</table>

### 🛡 Security & reliability

<table>
<tr><td width="220"><b>🔒 Tool approval + Docker sandbox</b></td><td>Sensitive tool calls enter a unified approval flow. High-risk commands and file writes can run inside a Docker sandbox to reduce the blast radius of privileged mistakes.</td></tr>
<tr><td><b>🏠 Local-first · zero third-party hops</b></td><td>Configuration, sessions, memory, attachments, skills, and logs live under <code>~/.hope-agent/</code> by default, while API keys connect directly to model providers. Daemon mode adds Bearer token authentication and SSRF protection policies.</td></tr>
<tr><td><b>🛟 Rollback, recovery, and keepalive</b></td><td>Configuration changes create local snapshots for one-click rollback. Guardian, system services, and subsystem watchdogs provide restart, diagnosis, and reconnection so long-running work recovers in an observable way.</td></tr>
</table>

> For the complete version history, see [CHANGELOG.md](CHANGELOG.md). Implementation details live under [docs/architecture/](docs/architecture/).

## Quick Start

### Install Locally

> 📦 Full installer list across platforms: [Releases](https://github.com/shiwenwen/hope-agent/releases)

#### macOS

##### Homebrew (recommended)

```bash
brew tap shiwenwen/hope-agent
brew install --cask hope-agent
```

> Already have `Hope Agent.app` installed manually? Append `--adopt` to let the cask take over your existing same-version app without re-downloading, or `--force` to overwrite.

##### Manual install (DMG)

Download `Hope.Agent_*.dmg` from [Releases](https://github.com/shiwenwen/hope-agent/releases) and drag into Applications.

> If macOS reports "damaged" or "cannot verify the developer" on first launch, run in Terminal:
>
> ```bash
> sudo xattr -cr /Applications/Hope\ Agent.app
> sudo codesign --force --deep --sign - /Applications/Hope\ Agent.app
> ```

Native builds for both Apple Silicon (arm64) and Intel (x64); Homebrew and manual download both pick the correct DMG for your hardware automatically.

##### Launch modes

- **Desktop GUI**: Launchpad / Applications folder (click the Hope Agent icon), or `open -a "Hope Agent"` / `hope-agent` from a terminal
- **Browser Web GUI**: open <http://127.0.0.1:8420> after launching the desktop app, or run `hope-agent server start` to start only the service without a desktop window
- **ACP (IDE integration)**: `hope-agent acp`

#### Windows

##### Scoop (recommended)

```powershell
scoop bucket add hope-agent https://github.com/shiwenwen/scoop-hope-agent
scoop install hope-agent
```

##### Manual install (installer)

Download `Hope.Agent_*-setup.exe` from [Releases](https://github.com/shiwenwen/hope-agent/releases) and double-click. **Windows is not yet fully tested** — please file issues if anything breaks.

> If Windows reports "MSVCP140_1.dll was not found" or a similar missing `VCRUNTIME140.dll` / `MSVCP140.dll` error on launch, install the [Microsoft Visual C++ 2015–2022 Redistributable (x64)](https://aka.ms/vs/17/release/vc_redist.x64.exe) and relaunch.

x64 only.

##### Launch modes

- **Desktop GUI**: click "Hope Agent" in the Start menu, or `hope-agent` from PowerShell
- **Browser Web GUI**: open <http://127.0.0.1:8420> after launching the desktop app, or run `hope-agent server start` in PowerShell / cmd to start only the service without a desktop window
- **ACP (IDE integration)**: `hope-agent acp`

#### Linux

##### Arch Linux / Manjaro (AUR)

```bash
yay -S hope-agent-bin   # or paru / any AUR helper
```

Pre-built binary package (repackaged from the GitHub Release `.deb`) — no source compilation.

##### Debian / Ubuntu (apt)

```bash
curl -fsSL https://shiwenwen.github.io/hope-agent-linux-repo/pubkey.gpg | \
  sudo gpg --dearmor -o /usr/share/keyrings/hope-agent.gpg
echo "deb [signed-by=/usr/share/keyrings/hope-agent.gpg] https://shiwenwen.github.io/hope-agent-linux-repo/apt stable main" | \
  sudo tee /etc/apt/sources.list.d/hope-agent.list
sudo apt update
sudo apt install hope-agent
```

##### Fedora / RHEL / CentOS (dnf / yum)

```bash
sudo curl -fsSL https://shiwenwen.github.io/hope-agent-linux-repo/rpm/hope-agent.repo \
  -o /etc/yum.repos.d/hope-agent.repo
sudo dnf install hope-agent     # or `sudo yum install hope-agent`
```

> The older `sudo dnf config-manager --add-repo …` form has been removed in dnf5 (Fedora 41+); the `curl` variant above works on dnf4 / dnf5 / yum / zypper alike.

openSUSE users:

```bash
sudo zypper addrepo https://shiwenwen.github.io/hope-agent-linux-repo/rpm/hope-agent.repo
sudo zypper install hope-agent
```

##### Manual install (AppImage / deb / rpm)

From [Releases](https://github.com/shiwenwen/hope-agent/releases) (filenames include the arch suffix — pick `_amd64` / `_arm64` for deb/AppImage or `.x86_64` / `.aarch64` for rpm):

- AppImage: `Hope.Agent_*.AppImage` — `chmod +x` and run
- Debian / Ubuntu: `Hope.Agent_*.deb` — `sudo dpkg -i Hope.Agent_*.deb`
- Fedora / RHEL: `Hope.Agent_*.rpm` — `sudo rpm -i Hope.Agent_*.rpm`

Both amd64 (x86_64) and arm64 (aarch64) native builds are published, covering desktops, Raspberry Pi 4/5, Apple Silicon Macs running Asahi Linux, and Graviton / Ampere cloud VMs. apt and dnf automatically pick the right arch via `dpkg --print-architecture` / `$basearch`.

##### Launch modes

- **Desktop GUI**: click "Hope Agent" in your app menu, or `hope-agent` from a terminal
- **Browser Web GUI**: open <http://127.0.0.1:8420> after launching the desktop app, or run `hope-agent server start` to start only the service without a desktop window
- **ACP (IDE integration)**: `hope-agent acp`

#### First launch & auto-update

1. First launch wizard: **pick a provider template → paste API key / sign in with Codex OAuth → chat.**
2. Desktop builds ship with the GitHub Releases auto-updater. Go to **Settings → About** in-app to check for and install updates, or just tell the model "upgrade" / "check for updates" in chat.
3. Versions installed via Homebrew / AUR / Scoop also receive updates through the built-in updater; the package manager's recorded version stays pinned to the initial install version and does not affect functionality.

> To connect from a phone or another computer, set an API key under **Settings → Server** and change the bind address to `0.0.0.0:8420`. After restarting, open `http://<IP of the device running Hope Agent>:8420`. Never expose the port to a LAN or the public internet without authentication; for public access, put it behind an HTTPS reverse proxy. See the [Docker deployment guide](docs/deployment/docker.en.md).

### Self-hosting (Docker)

For running Hope Agent on a home NAS / VPS / homelab and accessing the Web GUI from a browser:

```bash
docker run -d \
  --name hope-agent \
  -p 127.0.0.1:8420:8420 \
  -v hope-data:/data \
  ghcr.io/shiwenwen/hope-agent:latest
```

Once the container is running, open <http://127.0.0.1:8420> in a browser and follow the onboarding wizard to configure provider API keys. The image covers `linux/amd64` + `linux/arm64` (including Apple Silicon and Raspberry Pi) and is auto-built on every release tag.

For docker-compose, the optional Ollama sidecar for local LLMs, LAN exposure, reverse proxy + TLS, and upgrade flow, see [`docs/deployment/docker.en.md`](docs/deployment/docker.en.md).

### For developers

```bash
git clone https://github.com/shiwenwen/hope-agent.git
cd hope-agent
pnpm install
pnpm tauri dev         # desktop dev (frontend + Rust hot reload)

# Other useful commands
pnpm typecheck         # frontend typecheck (tsc -b)
pnpm lint              # lint
pnpm tauri build       # production build
```

For local Web GUI development with live reload, run `pnpm tauri dev` and open `http://localhost:1420` in your browser. That is the Vite dev server, sharing the same frontend HMR as the Tauri window. `http://localhost:8420` is the embedded HTTP/WS server's static Web GUI entry, served from `dist/` / the embedded bundle, so it behaves like the packaged browser entry and does not HMR with source changes. If your local server has an API key enabled, the `1420` page may get 401s from `8420`; for development, temporarily clear the Server API Key in Settings and restart.

## Run Modes

| Mode             | How to start                                                                      | When to use                                                                       |
| ---------------- | --------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| Desktop GUI      | Double-click the app / `pnpm tauri dev`                                        | The most complete entry point: full GUI plus an embedded HTTP/WS server, so the desktop can serve remote clients while you use it |
| Server + Web GUI (HTTP/WS) | `server start` subcommand; `server install` registers a launchd / systemd service | Headless always-on daemon for IM channels and cron jobs; **the React frontend is `rust-embed`-baked into the server binary, so opening `http://<server>:port` in any browser gives you the full Web GUI** — phone, tablet, any computer can connect directly without installing a client |
| ACP (stdio)      | `acp` subcommand                                                                  | IDE integration — any ACP-capable editor can call Hope Agent as its agent backend |

All three modes share the same `ha-core` core. Config, sessions, and memories live under `~/.hope-agent/`.

## Ecosystem

<table>
<tr>
  <td width="140"><b>📦 Model providers</b></td>
  <td>
    <b>40+ templates · 300+ preset models</b><br/>
    <b>International</b> · Anthropic · OpenAI · Codex · GitHub Copilot · Google Gemini · OpenRouter · Azure OpenAI · Groq · Together AI · Fireworks · Novita · Perplexity · xAI Grok · Mistral · Cohere<br/>
    <b>China</b> · DeepSeek · Moonshot (Kimi) · Qwen · Doubao (Volcengine) · Z.AI (GLM) · MiniMax · Xiaomi MiMo<br/>
    <b>Local</b> · Ollama · any OpenAI-compatible endpoint
  </td>
</tr>
<tr>
  <td><b>💬 IM channels</b></td>
  <td><b>10+</b> · Telegram · Discord · Slack · Feishu · Google Chat · LINE · QQ Bot · Signal · iMessage · IRC · WeChat · WhatsApp</td>
</tr>
<tr>
  <td><b>🌐 UI languages</b></td>
  <td><b>10+</b> · Simplified Chinese · Traditional Chinese · English · Japanese · Korean · Spanish · Portuguese · Russian · Arabic · Turkish · Vietnamese · Malay</td>
</tr>
</table>

## Project Structure

Cargo workspace, three crates; all business logic lives in `ha-core`:

```
crates/
  ha-core/       Rust core library (zero Tauri deps) — where the logic lives
  ha-server/     axum HTTP/WS daemon (thin shell)
src-tauri/       Tauri desktop shell (thin shell)
src/             React 19 + TypeScript frontend
skills/          Bundled skills (ship with the app)
```

For the full module map, architecture conventions, and coding guidelines, see [AGENTS.md](AGENTS.md).

## Documentation

- 📖 **[User Guide](docs/user-guide/en/README.md)** — the complete user manual: installation, getting started, and how to use and configure every feature ([简体中文](docs/user-guide/README.md))
- 🏗️ [Technical docs](docs/) — architecture and implementation details (for developers)

## Contributing

The main branch is under active development — issues and PRs are welcome. Please skim the **Architecture** and **Coding Conventions** sections of [AGENTS.md](AGENTS.md) before contributing.

Common commands:

```bash
pnpm tauri dev                    # desktop dev
cargo check --workspace              # Rust dep / type check
cargo test -p ha-core -p ha-server   # core tests
node scripts/sync-i18n.mjs --check   # i18n completeness check
```

## Community

- 🐛 [Issues](https://github.com/shiwenwen/hope-agent/issues) — bug reports, feature requests
- 💡 [Discussions](https://github.com/shiwenwen/hope-agent/discussions) — usage, ideas, Q&A
- ⭐ If Hope Agent helps you, consider giving it a star on GitHub
- 📮 Roadmap, a dedicated docs site, and more community channels are on the way

## Acknowledgements

- [Ollama](https://ollama.com/) — the one-click local LLM experience is built on top of Ollama's local runtime and its OpenAI-compatible endpoint; Hope Agent only wraps the GUI layer, while Qwen / Gemma and other models are distributed through the Ollama model library
- [ClawHub](https://www.clawhub.com/) / [SkillHub](https://skillhub.cn/) — public skill discovery sources for Hope Agent
- [Tauri](https://tauri.app/), [axum](https://github.com/tokio-rs/axum), [React](https://react.dev/), [shadcn/ui](https://ui.shadcn.com/), [Streamdown](https://github.com/streamdown/streamdown), [Radix UI](https://www.radix-ui.com/), and the rest of the open source stack Hope Agent stands on
- Everyone who has filed issues, tested builds, and given feedback along the way

## Star History

<a href="https://www.star-history.com/#shiwenwen/hope-agent&Date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=shiwenwen/hope-agent&type=Date&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=shiwenwen/hope-agent&type=Date" />
    <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=shiwenwen/hope-agent&type=Date" />
  </picture>
</a>

## License

[MIT](LICENSE)
