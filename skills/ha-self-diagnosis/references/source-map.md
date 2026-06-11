# Hope Agent Source Map

Use this as a fallback map when the live source tree is not available. Prefer
live files and `docs/architecture/` whenever possible — the architecture docs
are the single source of truth and this map only points at them.

## Runtime Forms

- Desktop GUI: Tauri 2 shell in `src-tauri/`, React frontend in `src/`, business logic in `crates/ha-core/`.
- HTTP/WS daemon: `crates/ha-server/`, using axum routes and shared `ha-core`.
- ACP stdio: `hope-agent acp`, sharing core agent/session/runtime behavior.

Detect the active mode with `ha_core::runtime_role()` / `is_desktop()`.

## Core Contracts

- Business logic belongs in `crates/ha-core/` (zero Tauri deps); Tauri and server crates are thin adapters.
- Frontend calls go through `src/lib/transport.ts`; every new command needs both Tauri and HTTP implementations.
- State is centered on `CoreState`, SQLite databases under `~/.hope-agent/`, and EventBus broadcasts.
- Config reads use `config::cached_config()`; config writes use `config::mutate_config((category, source), …)` (never manual load+save / `Mutex<AppConfig>`).
- Provider list / `active_model` writes go through `provider/crud.rs` helpers (never `providers.push`/`retain`).
- Every LLM request goes through `failover::executor::execute_with_failover` (3 policies); Codex never rotates profiles.
- Outbound HTTP/WS must pass `security::ssrf::check_url`; never hand-write IP checks.
- Note/knowledge writes use `crate::platform::write_atomic` (never `fs::write`); secrets use `platform::write_secure_file` (0600).
- Knowledge-base access is deny-by-default through `effective_kb_access`; owner plane (HTTP/Tauri) and agent plane (`note_*` tools) are physically isolated.
- HTTP preview-by-path is gated by `authorized_canonical_file_path` — remote callers can never read arbitrary host paths.
- Tool visibility (`dispatch::resolve_tool_fate`) and execution gating (`permission::engine::resolve_async`) are separate; hiding a tool is never a security boundary.
- Settings rollback / autosave snapshots live under `~/.hope-agent/backups/autosave/`.

## Important Directories

### Backend (`crates/ha-core/src/`)

- `tools/`: tool definitions (`definitions/core_tools.rs`), `dispatch.rs` (visibility), `execution.rs`; `tools/settings.rs` is the `ha-settings` surface; `tools/note.rs`, `tools/browser/`, `tools/canvas/`, `tools/image_generate/`, `tools/ask_user_question.rs`, `tools/app_update.rs`.
- `chat_engine/`: chat loop (`engine.rs`), streaming sinks, `im_mirror.rs`, `finalize/`, round accumulator.
- `session/`: session/message/task SQLite (`db.rs`); `artifacts.rs` (workspace aggregation), `helpers.rs` (effective working dir), `subagent_db.rs`.
- `config/`: persisted `AppConfig`, `cached_config()` / `mutate_config()`, `persistence.rs`.
- `provider/`: templates, `crud.rs` write helpers, `local.rs` known local backends.
- `failover/`: `executor.rs` (`execute_with_failover`), profile rotation; `agent/side_query.rs`.
- `context_compact/`: 5-tier progressive compaction (`compact.rs`), `ContextEngine` / `CompactionProvider`.
- `memory/` + `memory_extract.rs`: retrieval/extraction (Project > Agent > Global); vec0 gated on `memory_embedding`.
- `knowledge/`: `service.rs` (owner plane), `access.rs` (`effective_kb_access`), `index.rs`, `search.rs`, `registry.rs`, `sprite/`, `maintenance/`.
- `channel/`: IM plugins, `worker/` (`dispatcher.rs`, `streaming.rs`, `eviction_watcher.rs`, `ask_user.rs`), `start_watchdog.rs`, `db.rs`.
- `hooks/`: `mod.rs` (`HookDispatcher::dispatch`), `scopes.rs` (four-layer scope), `runner/`, `matcher.rs`, `decision.rs`, `parse.rs`.
- `permission/` + `permissions.rs`: unified permission engine v2 (`engine.rs::resolve_async`).
- `security/`: `ssrf.rs`, `dangerous.rs` (global YOLO / dangerous-skip).
- `agent/`: `resolver.rs`, `preflight.rs` (`UserPromptSubmit`), `migration.rs`, `side_query.rs`, `context.rs`, KB access resolution.
- `plan/`: Plan Mode 5-state machine.
- `subagent/` (`injection.rs`), `team/` (`coordinator.rs`), `cron/` (`scheduler.rs`, `executor.rs`).
- `dashboard/` (`insights.rs`), `recap/`, `awareness/`: analytics, deep recap, cross-session behavior awareness.
- `ask_user/`, `system_prompt/` (`build.rs`, `constants.rs`), `skills/` (discovery, authoring, `mention.rs`).
- `browser/` + `browser_state.rs`, `mac_control.rs`: browser automation (8-action, dual backend) and macOS control.
- `filesystem/`: `workspace.rs` (`WorkspaceScope`), `ops.rs`.
- `updater/`: `keys.rs`, `download.rs`, `self_contained.rs`, `auto_check.rs`, `backup.rs`, `staging.rs`.
- `guardian.rs`, `self_diagnosis.rs`, `crash_journal.rs`: reliability / crash self-heal.
- `local_llm/` + `local_model_jobs.rs`, `docker/` (SearXNG sandbox), `platform/`, `logging/`, `mcp/`, `acp/`, `slash_commands/`.

### Frontend (`src/`)

- `lib/transport.ts`, `lib/transport-tauri.ts`, `lib/transport-http.ts`: transport adapters.
- `components/chat/`: chat UI, `workspace/` (artifacts panel), `files/` (unified file actions + office preview), `diff-panel/`, `project/file-browser/` (`FilePreviewPane`).
- `components/settings/`, `components/dashboard/`, `components/cron/`: settings panels and analytics UI.

### Adapters

- `src-tauri/src/commands/`: Tauri command wrappers; registered in `src-tauri/src/lib.rs` `invoke_handler!`.
- `crates/ha-server/src/routes/` + `router.rs`: HTTP/WS route wrappers.

## Architecture Docs

`docs/architecture/` is the source of truth (see `docs/README.md` and
`overview.md` for the canonical index). Grouped by area:

### System architecture

- `overview.md` — whole-system map, storage table, module dependencies.
- `backend-separation.md`, `transport-modes.md`, `process-model.md` — three-crate split, EventBus, run modes, Guardian.
- `api-reference.md` — Tauri ↔ HTTP command parity.

### Core modules

- `chat-engine.md`, `provider-system.md`, `failover.md`, `side-query.md`, `local-model-loading.md`.
- `prompt-system.md`, `tool-system.md`, `permission-system.md`, `context-compact.md`.
- `session.md`, `project.md`, `memory.md`, `knowledge-base.md`.

### Agent capabilities

- `plan-mode.md`, `ask-user.md`, `skill-system.md`, `subagent.md`, `agent-team.md`, `behavior-awareness.md`.

### Access layer

- `im-channel.md`, `acp.md`, `slash-commands.md`, `mcp.md`.

### Infrastructure

- `image-generation.md`, `cron.md`, `sandbox.md`, `dashboard.md`, `recap.md`, `logging.md`.
- `reliability.md` (Guardian / crash journal), `config-system.md`, `security.md`, `platform.md`.
- `browser.md`, `macos-control.md`, `canvas.md`, `file-operations.md`, `self-update.md`, `self-diagnosis-issue-reporting.md`.

## Runtime Databases

All under `~/.hope-agent/`, centralized in `paths.rs`. **Always open read-only during diagnosis.**

| Database | Purpose |
|----------|---------|
| `sessions.db` | Sessions, messages, tasks, `chat_turns`, subagent/ACP/team runs, `learning_events`, `channel_conversations`, `ask_user_questions`, KB registry + access bindings |
| `memory.db` | Memory entries + FTS5 (`memories_fts`) + vec0 (`memories_vec`) + embedding cache |
| `knowledge/index.db` | Knowledge-base chunk index (FTS5 + vec0); rebuildable cache — notes' truth is the `.md` files in `knowledge/{id}/notes/` or external vaults |
| `logs.db` | Structured app logs (query/filter) |
| `cron.db` | `cron_jobs` + `cron_run_logs` |
| `async_jobs.db` | Async tool jobs (exec / web_search / image_generate backgrounded) |
| `local_model_jobs.db` | Local model install / pull background jobs |
| `local_llm_library_cache.db` | Ollama Library search / tag metadata cache (24h TTL) |
| `recap/recap.db` | Cached deep-recap reports / facets |
| `canvas/canvas.db` | `canvas_projects` + `canvas_versions` (orphans persist; no session FK cascade) |

Non-DB state: `config.json` (providers, model chain, global settings), `agents/{id}/agent.json` (per-agent config), `projects/{id}/` (project working dirs / real files), `credentials/` (OAuth + MCP creds, 0600), `plans/`, `channels/`, `attachments/`, `permission/*.json`, `crash_journal.json`, `mac-control/`, `browser-profiles/`.
