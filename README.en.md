<p align="center">
  <img src="assets/logo.png" alt="Hope Agent" width="200">
</p>

<h1 align="center">Hope Agent</h1>

<p align="center">
  <strong>A desktop-first AI assistant deeply integrated with your OS — also runs headless, on a NAS or in the cloud.</strong><br/>
  Remembers you · Grows over time · Reachable from every chat app you use
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

**Hope Agent** is an AI assistant built for ordinary people — we put the weight on **a first-class desktop GUI deeply integrated with the OS**, while it also runs as a background service that lives happily on a NAS, home server, or cloud VM, and stays reachable through your IM apps. One native installer, GUI templates for the major model providers baked in, paste an API key and you're chatting. It remembers you across sessions, quietly organizes its own memory while idle, and turns completed tasks into reusable skills.

## Why Hope Agent

Ordinary people deserve an AI assistant that just **opens and works** — download an installer, double-click, no runtimes to install, no CLI to learn first, no cryptic config to decipher, no service quietly crashing at 3am with no one to fix it. Hope Agent isn't only a desktop app — it also runs as an HTTP/WS service you can park on a NAS, home server, or cloud VM and leave running 24/7, while it hooks into IM channels and talks to IDEs over ACP — but we believe the door most people walk through is still the desktop, so that's where we put the most effort: **a first-class desktop GUI deeply integrated with the OS**, polished together with performance, stability, and the small interaction details.

Hope Agent was influenced in its early days by [openclaw](https://github.com/openclaw/openclaw) — credit to them for their pioneering work on local AI assistants. We took a different implementation path.

## Highlights

### 🎯 Everyday use

<table>
<tr><td width="220"><b>🖥️ Native desktop GUI</b></td><td>Native macOS / Linux / Windows app, ready to run out of the installer. Ships in 12 UI languages (Simplified/Traditional Chinese, English, Japanese, Korean, Spanish, Portuguese, Russian, Arabic, Turkish, Vietnamese, Malay) with a polished dark theme and carefully tuned typography.</td></tr>
<tr><td><b>🧙 Zero-config providers</b></td><td>36 built-in provider templates covering 166 preset models. Anthropic, OpenAI, Gemini, Codex, OpenRouter, DeepSeek, Kimi, Qwen, Doubao, GLM, MiniMax, xAI, Mistral, Ollama — all in. Each provider supports multi-key rotation, so rate limits and quota exhaustion fail over seamlessly to the next key.</td></tr>
<tr><td><b>🦙 One-click local models</b></td><td><b>No account, no API key, no terminal</b> — Settings → Model picks a Qwen3.6 / Gemma 4 size that fits your hardware, then handles <a href="https://ollama.com">Ollama</a> install, model pull, provider registration, and active-model switch in one click. Same flow covers local embedding models too.</td></tr>
<tr><td><b>💬 One app, every chat</b></td><td>12 IM channels: Telegram, Discord, Slack, Feishu, Google Chat, LINE, QQ Bot, Signal, iMessage, IRC, WeChat, WhatsApp. Inbound images / voice / files become multimodal context automatically; tool approvals are one tap in the chat window; every group / account can bind a distinct Agent with its own policies.</td></tr>
<tr><td><b>🌐 Standalone service · browser is the client</b></td><td><b>Not just a desktop app</b> — Hope Agent can run fully headless as a service. One command <code>hope-agent server start</code> launches an HTTP/WS daemon; <code>server install</code> registers it as a launchd / systemd auto-start unit so it lives 24/7 on your NAS, cloud VM, or spare laptop. <b>The server ships an embedded Web GUI</b> (the React frontend is baked into the binary via <code>rust-embed</code>) — <b>point any browser on your phone, tablet, or another computer at <code>http://&lt;server&gt;:port</code> and you get the full React UI</b>, no client install, no separate frontend deployment. Bearer token auth and three-tier SSRF policies keep public exposure controlled. Sessions, memories, cron jobs, and IM channels all run server-side — the client is just a window.</td></tr>
<tr><td><b>🔁 Three run modes, one core</b></td><td>Desktop GUI (default), HTTP/WS daemon with embedded Web GUI (browser-direct), and ACP stdio (as an agent backend for any ACP-capable IDE). All three share a pure-Rust <code>ha-core</code> library with zero Tauri dependencies — the same code is a desktop app, a server, and an IDE backend.</td></tr>
</table>

### 🧠 Memory & learning

<table>
<tr><td width="220"><b>🧠 Persistent memory across sessions</b></td><td>SQLite + FTS5 + vector search, three-in-one. Memories are scoped by Global / Project / Agent; system prompt injection follows a joint budget so no one layer crowds out another.</td></tr>
<tr><td><b>🕶 Incognito chat</b></td><td>A per-session switch that can apply from the very first message. When enabled, the current chat gets no passive memory or cross-session awareness injection and does not auto-collect memory; memory tools are only used when you explicitly ask it to remember or recall something.</td></tr>
<tr><td><b>💤 Offline "dreaming"</b></td><td>When idle, Hope Agent automatically reviews "what was worth remembering over the past couple of days," pins the selections, and writes them into a markdown diary viewable under Settings → Dream Diary. Every day's work gets quietly consolidated for next time.</td></tr>
<tr><td><b>🔍 Active recall + reflective profile</b></td><td>Before each turn starts, the most relevant memories for your input are pulled into the prompt (Active Memory). A separate reflective pass distills your communication style, work habits, and long-term preferences into a dedicated "User Profile" section — it gets better at knowing you over time.</td></tr>
<tr><td><b>🛠 Skills that grow</b></td><td>After complex tasks, Hope Agent auto-drafts new skills for your review. Approve a draft in settings and it's reusable from then on. Skills support conditional activation (e.g. only load when editing Python files), forked sub-agent execution, and tool allowlists; compatible with the <a href="https://agentskills.io">agentskills.io</a> open standard.</td></tr>
<tr><td><b>👁 Cross-session awareness</b></td><td>It knows what your other chats are doing. Before each turn, Hope Agent pulls in the recent goals, actions, and friction points of your other active sessions — so when context crosses over, the right information is available without derailing the main conversation. Defaults to a zero-LLM-cost structured mode; an optional LLM digest mode is available.</td></tr>
<tr><td><b>💾 Long conversations don't lose the plot</b></td><td>Five-tier progressive context compaction. No matter how long the chat, earlier messages aren't hard-truncated. Tool calls stay paired forever; when messages are summarized, recently edited file contents are auto-restored from disk so you don't have to paste them again. Combined with prompt caching, long-session API costs stay well below naive usage.</td></tr>
</table>

### 🛠 Workflow & tools

<table>
<tr><td width="220"><b>📋 Plan Mode</b></td><td>For complex tasks, Hope Agent first drafts an editable, resumable plan managed by a five-state machine, with plan files physically isolated per agent / session so cross-session bleed can't happen. Plans persist across sessions — "continue the previous plan" is enough to pick up. During execution, it strictly respects a tool allowlist so the model can't wander.</td></tr>
<tr><td><b>📁 Project containers</b></td><td>Group related sessions under a single project that inherits project-level memory / instructions / shared files. Uploaded files get automatic text extraction and three-layer injection (dir listing / small-file auto-inline / large-file on-demand read) — no manual @ file, no context blowup.</td></tr>
<tr><td><b>👥 Agent teams</b></td><td>Pre-configure team templates in settings (member roles, bound agents, default task templates). One sentence tells the model to spin up a specialist team. Members message each other and coordinate; when done, the transcript is summarized back to the main thread.</td></tr>
<tr><td><b>🗓 Natural-language cron</b></td><td>"Write me a daily summary every 8 AM." "Review last week's todos every Monday." "Scan my inbox hourly on workdays." Scheduled in plain language, delivered to any IM channel. Runs reliably under both desktop GUI and the daemon.</td></tr>
<tr><td><b>📊 Dashboard + Recap</b></td><td>Built-in analytics: cost, token usage, activity heatmap, and a four-dimensional health score. <code>/recap</code> runs a deep retrospective over the last N days and produces an 11-section AI report (Agent tool optimization, memory &amp; skill recommendations, cost optimization, etc.) that exports as standalone HTML.</td></tr>
<tr><td><b>🔌 MCP client (OAuth 2.1)</b></td><td>Built-in Model Context Protocol client with all four transports: stdio / Streamable HTTP / SSE / WebSocket. Full OAuth 2.1 + PKCE flow (automatic discovery, RFC 7591 dynamic client registration, loopback callback) persists credentials at 0600 on disk; standards-compliant OAuth servers like Notion / Linear work with a single click. All outbound URLs pass through the SSRF policy. One-click import from <code>claude_desktop_config.json</code> in Settings; tools surface as <code>mcp__&lt;server&gt;__&lt;tool&gt;</code> in the main conversation, with extra <code>mcp_resource</code> / <code>mcp_prompt</code> tools for passive data. Long-running tools auto-background.</td></tr>
<tr><td><b>🔧 Toolbox</b></td><td>Controllable browser (CDP), Canvas, AI image generation (7 providers), web search (8 providers with failover), bash (optional Docker sandbox), file read/grep/find, URL preview, crash journal, self-diagnosis.</td></tr>
<tr><td><b>⚡ Background long tasks</b></td><td>Long-running shell commands, web searches, or image generations can be "sent to the background" — an immediate <code>job_id</code> returns so the conversation keeps flowing. The result is auto-injected back into the main thread when it finishes; the model can also poll with <code>job_status</code> on demand. No task is ever long enough to freeze your chat window.</td></tr>
</table>

### 🛡 Security & local-first

<table>
<tr><td width="220"><b>🔒 Tool approval + Docker sandbox</b></td><td>Sensitive tool calls are gated by an approval flow (with per-category auto deny / proceed timeouts and per-channel auto-approve). High-risk bash / file writes can be routed into an isolated Docker sandbox. Safe to give the Agent high privileges.</td></tr>
<tr><td><b>🏠 Local-first · zero third-party hops</b></td><td>All data lives under <code>~/.hope-agent/</code>: config, sessions, memories, attachments, skills, logs — all local SQLite / files. API keys hit model providers directly. In daemon mode, Bearer token auth plus three-tier SSRF policies keep remote access controllable.</td></tr>
<tr><td><b>🛟 Automatic config snapshots · one-click rollback</b></td><td>Every config write auto-snapshots to <code>backups/autosave/</code>, keeping the last 50. Even if the model's settings tool garbles your preferences, you can restore to any previous state.</td></tr>
<tr><td><b>♻️ Crash self-healing · 3-layer keepalive</b></td><td>A parent–child Guardian process supervises the app and auto-restarts it with exponential backoff (1s → 3s → 9s → 15s → 30s) on unexpected exit; after 5 consecutive crashes it snapshots the config, runs an LLM self-diagnosis and attempts automatic remediation — crash history is browsable under <i>Settings → Crash History</i>. Once <code>server install</code> registers the daemon, launchd <code>KeepAlive</code> / systemd <code>Restart=on-failure</code> add an OS-level second layer — even if the Guardian itself is <code>kill -9</code>'d, the OS brings it back. Cron, IM channels and MCP connections each run their own watchdogs with auto-reconnect.</td></tr>
</table>

> For the full list of built-in features, see [CHANGELOG.md](CHANGELOG.md).

## Quick Start

### For users

> 🛠 **Release status**: 0.1.0 is usable but we haven't shipped binaries yet — we want one more polish pass on the first-run experience before we do. The code is ready whenever you are; if you're curious, `pnpm tauri build` produces the exact same artifact the Release will. See **For developers** below.

1. Download the installer for your platform from [Releases](https://github.com/shiwenwen/hope-agent/releases):
   - macOS: `Hope-Agent_*.dmg`
   - Linux: `hope-agent_*.AppImage`
   - Windows: `Hope-Agent_*.exe` / `Hope-Agent_*.msi` (not yet fully tested)
2. First launch: **pick a provider template → paste API key / sign in with Codex OAuth → chat.**
3. Desktop installers ship with GitHub Releases auto-update; inside the app you can go to **Settings → About** to check and install updates

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
    <b>36 templates · 166 preset models</b><br/>
    <b>International</b> · Anthropic · OpenAI · Codex · Google Gemini · OpenRouter · Azure OpenAI · Groq · Together AI · Fireworks · Perplexity · xAI Grok · Mistral · Cohere<br/>
    <b>China</b> · DeepSeek · Moonshot (Kimi) · Qwen · Doubao (Volcengine) · Z.AI (GLM) · MiniMax · Xiaomi MiMo<br/>
    <b>Local</b> · Ollama · any OpenAI-compatible endpoint
  </td>
</tr>
<tr>
  <td><b>💬 IM channels</b></td>
  <td><b>12</b> · Telegram · Discord · Slack · Feishu · Google Chat · LINE · QQ Bot · Signal · iMessage · IRC · WeChat · WhatsApp</td>
</tr>
<tr>
  <td><b>🌐 UI languages</b></td>
  <td><b>12</b> · Simplified Chinese · Traditional Chinese · English · Japanese · Korean · Spanish · Portuguese · Russian · Arabic · Turkish · Vietnamese · Malay</td>
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

See [docs/](docs/).

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

- [openclaw](https://github.com/openclaw/openclaw) — inspiration in the local AI assistant space
- [Ollama](https://ollama.com/) — the one-click local LLM experience is built on top of Ollama's local runtime and its OpenAI-compatible endpoint; Hope Agent only wraps the GUI layer, while Qwen / Gemma and other models are distributed through the Ollama model library
- [ClawHub](https://www.clawhub.com/) / [SkillHub](https://skillhub.cn/) — public skill discovery sources for Hope Agent
- [Hermes Agent](https://github.com/NousResearch/hermes) (originally adapted from [obra/superpowers](https://github.com/obra/superpowers)) — several bundled coding-methodology skills are vendored from here (MIT); see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)
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
