# Hope Agent Source Map

Use this as a fallback map when the live source tree is not available. Prefer live files and `docs/architecture/` whenever possible.

## Runtime Forms

- Desktop GUI: Tauri 2 shell in `src-tauri/`, React frontend in `src/`, business logic in `crates/ha-core/`.
- HTTP/WS daemon: `crates/ha-server/`, using axum routes and shared `ha-core`.
- ACP stdio: `hope-agent acp`, sharing core agent/session/runtime behavior.

## Core Contracts

- Business logic belongs in `crates/ha-core/`; Tauri and server crates are thin adapters.
- Frontend calls go through `src/lib/transport.ts`; every new command needs Tauri and HTTP implementations.
- State is centered on `CoreState`, SQLite databases under `~/.hope-agent/`, and EventBus broadcasts.
- Outbound HTTP must pass SSRF checks through `security::ssrf::check_url`.
- Config writes should use `config::mutate_config`; provider writes should use `provider/crud.rs`.
- Settings rollback snapshots live under `~/.hope-agent/backups/autosave/`.

## Important Directories

- `docs/architecture/`: source of truth for subsystem behavior.
- `crates/ha-core/src/tools/`: tool definitions and execution handlers.
- `crates/ha-core/src/chat_engine/`: chat loop, streaming, IM mirror, finalization.
- `crates/ha-core/src/session/`: session/message/task SQLite persistence.
- `crates/ha-core/src/channel/`: IM channel plugins, worker, dispatcher, streaming.
- `crates/ha-core/src/config/`: persisted `AppConfig` and cache/mutation helpers.
- `crates/ha-core/src/skills/`: skill discovery, prompt catalog, fork helper, authoring.
- `src/components/settings/`: desktop/web Settings panels.
- `src/lib/transport-http.ts` and `src/lib/transport-tauri.ts`: frontend transport adapters.
- `src-tauri/src/commands/`: Tauri command wrappers.
- `crates/ha-server/src/routes/`: HTTP route wrappers.

## Common Architecture Docs

- Process and crate boundaries: `process-model.md`, `backend-separation.md`, `transport-modes.md`.
- Tools and permissions: `tool-system.md`, `permission-system.md`.
- Skills: `skill-system.md`.
- Chat and streaming: `chat-engine.md`.
- Context compaction: `context-compact.md`.
- Memory: `memory.md`.
- IM channels: `im-channel.md`.
- Plan Mode: `plan-mode.md`.
- MCP: `mcp.md`.
- Self update: `self-update.md`.
- Logging: `logging.md`.
- Self diagnosis and GitHub issue reporting: `self-diagnosis-issue-reporting.md`.

## Runtime Databases

- `~/.hope-agent/logs.db`: app logs.
- `~/.hope-agent/sessions.db`: sessions, messages, tasks, subagent runs, learning events.
- `~/.hope-agent/async_jobs.db`: async tool jobs.
- `~/.hope-agent/cron.db`: recurring jobs.
- `~/.hope-agent/recap/recap.db`: cached recap reports.

Always open SQLite databases read-only during diagnosis.
