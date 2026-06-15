# Diagnostic Playbook

## Bug or Failure

1. Capture the user's symptom in one sentence.
2. Determine run mode: desktop, server, ACP, IM channel, cron, subagent, or MCP.
3. Locate the subsystem in the **Subsystem Reference** below — it names the doc, entry module, the right database, and the stable log category to grep.
4. Check settings relevant to the subsystem with `get_settings`.
5. Query recent logs read-only (the log `category` for each subsystem is in the reference):

```bash
sqlite3 -readonly -cmd ".headers on" -cmd ".mode column" ~/.hope-agent/logs.db \
  "SELECT timestamp, level, category, source, message, details
     FROM logs
    WHERE level IN ('ERROR','WARN')
    ORDER BY timestamp DESC
    LIMIT 80;"
```

   Narrow by subsystem with `AND category = '<category>'` (e.g. `knowledge`, `hooks`, `browser`, `failover`, `cron`).

6. If the issue is session-specific, inspect messages/tool rows:

```bash
sqlite3 -readonly -cmd ".headers on" -cmd ".mode column" ~/.hope-agent/sessions.db \
  "SELECT timestamp, role, tool_name, is_error, substr(content,1,240) AS content,
          substr(tool_result,1,240) AS tool_result
     FROM messages
    WHERE session_id = '<SESSION_ID>'
    ORDER BY timestamp DESC
    LIMIT 80;"
```

7. Map symptoms to source files using the **Subsystem Reference** and `docs/architecture/`.
8. Check whether a newer release may already fix it with `app_update(action="check")` when appropriate.
9. For issue reporting, include only relevant, redacted evidence.

## Subsystem Reference

Per subsystem: the architecture doc, the diagnostic entry module(s), the
runtime state (DB / config), the stable `logs.db` `category` to grep, and the
gotcha that most often explains a failure. Open every DB read-only.

### Knowledge Base (知识空间) — `knowledge-base.md`

- Entry: `knowledge/access.rs` (access), `knowledge/service.rs` (owner plane), `knowledge/search.rs`, `tools/note.rs`.
- State: `knowledge/index.db` (rebuildable cache); registry + attach bindings in `sessions.db` (`knowledge_bases` / `session_knowledge_bases` / `project_knowledge_bases`). Config: `knowledge_embedding`, `knowledge_search`, `knowledge_passive_recall`.
- Grep: `category='knowledge'` (sources: `service` / `index` / `reembed` / `embedding` / `maintenance::cycle`).
- Gotcha: access is deny-by-default via `effective_kb_access`; an agent that "can't see notes" usually has no attach — `SELECT kb_id,access FROM session_knowledge_bases WHERE session_id=?`. Empty search results → check `knowledge_embedding.enabled` (vector search degrades to FTS-only, never falls back to memory). Index drift → compare on-disk `.md` count vs `SELECT count(*) FROM note WHERE kb_id=?`.

### Hooks — `hooks.md`

- Entry: `hooks/mod.rs` (`HookDispatcher::dispatch`), `hooks/scopes.rs`, `hooks/runner/`, `hooks/parse.rs`; block points `agent/preflight.rs` (UserPromptSubmit) and `tools/execution.rs::fire_pre_tool_use_hook` (PreToolUse).
- State: `AppConfig.hooks`, `disable_all_hooks` (kill switch), `hooks_allow_project_scope` (gates project/local `.hope-agent/hooks.json`, default false).
- Grep: `category='hooks'` (source `dispatch` logs event/handler-count/decision/duration).
- Gotcha: project/local hooks not firing → `hooks_allow_project_scope` is off by default. Matcher normalizes Claude aliases (`Bash`→`exec`) but the payload `.tool_name` stays the internal name, so a script matching `.tool_name=="Bash"` never fires. Only `UserPromptSubmit`/`PreToolUse`/`PreCompact` actually block; the other 21 events are observation-only.

### Chat Engine / Session / Compaction / Memory — `chat-engine.md`, `session.md`, `context-compact.md`, `memory.md`

- Entry: `chat_engine/engine.rs`, `chat_engine/finalize/`, `session/db.rs`, `context_compact/compact.rs`, `memory/sqlite/`, `memory_extract.rs`.
- State: `sessions.db` (`context_json` snapshot, `messages.stream_status`, `chat_turns`), `memory.db`. Config: `compact.*` (`cacheTtlSecs=300`, override at usage ≥0.95), `memoryExtract.*`, `memory_embedding`.
- Grep: `category IN ('chat_engine','memory')`; compaction logs under `category='context'` (source `compact`) plus `category='agent'` (`reactive_microcompact`).
- Gotcha: Anthropic 400 on tool_use/tool_result → API-round (`_oc_round`) boundary broke; `prepare_messages_for_api()` strips metadata. Leftover partials → `SELECT id,stream_status FROM messages WHERE stream_status IN ('streaming','orphaned')`; stale turns → `SELECT status,interrupt_reason FROM chat_turns WHERE status IN ('running','cancelling')`. `recall_memory` excludes Project scope by design. `memory_embedding.enabled=false` → recall is FTS5-only.

### Provider / Failover / Side-query / Local LLM — `provider-system.md`, `failover.md`, `side-query.md`, `local-model-loading.md`

- Entry: `failover/executor.rs` (`execute_with_failover`), `chat_engine/engine.rs`, `agent/side_query.rs`, `provider/crud.rs`, `local_llm/management.rs`.
- State: `config.json` (`providers` / `active_model` / `fallback_models`); `sessions.db` (`provider_id` / `model_id` pin, `context_json`); `local_model_jobs.db`, `local_llm_library_cache.db`; `credentials/auth.json` (Codex OAuth).
- Grep: `source='failover'` (profile_rotation / codex_auth_expired / model_fallback); `category IN ('local_llm','local_model_jobs')`.
- Gotcha: model precedence is `plan_model > model_override > session pin > agent.primary > active_model`. Codex is force-excluded from profile rotation. OpenAIResponses must use `store:false` and never replay reasoning items (`rs_*` 404s) — check `context_json` for leaked `type:reasoning` items.

### Tools / Permissions / MCP / IM Channel / Skills / Logging — `tool-system.md`, `permission-system.md`, `mcp.md`, `im-channel.md`, `skill-system.md`, `logging.md`

- Entry: `tools/dispatch.rs` (visibility), `tools/execution.rs`, `permission/engine.rs` (`resolve_async`), `mcp/invoke.rs`, `channel/worker/dispatcher.rs`, `logging/db.rs`.
- State: `sessions.db` (`channel_conversations`, `sessions.permission_mode`, `learning_events`), `background_jobs.db`, `permission/*.json`, `credentials/mcp/{id}.json`. Config: `mcp_global.enabled`, `permission.global_yolo`, `deferredTools.enabled`.
- Grep: `category IN ('tool','mcp','channel','skills','permission','async_jobs')` (permission decisions log under `permission`; background-job lifecycle logs under `async_jobs`).
- Gotcha: tool not callable → trace `resolve_tool_fate` (visibility) then `resolve_async` (Decision); check `sessions.permission_mode` (`default|smart|yolo`) and `permission.global_yolo`. Hiding a tool is never a security boundary. MCP handshake 401/403 → `ServerState::NeedsAuth` (no retry loop). IM not replying / wrong session → `SELECT * FROM channel_conversations WHERE session_id=?` (1:1 attach both ways). **Background jobs (R1 naming split)**: the DB is `background_jobs.db` (table `background_jobs`, `JobKind` tool/subagent/group — only `tool` populated today) and the single code entry is `async_jobs::JobManager`, but the Rust **module + log category stay `async_jobs`** (lineage) — so grep job lifecycle under `category='async_jobs'`, not `background_jobs`. On upgrade the legacy `async_jobs.db` / `async_jobs/` are best-effort deleted (pure rebuildable cache, no migration).

### Subagent / Team / Cron — `subagent.md`, `agent-team.md`, `cron.md`

- Entry: `subagent/injection.rs`, `team/coordinator.rs`, `cron/scheduler.rs`, `cron/executor.rs`, `session/subagent_db.rs`, `wakeup/mod.rs` (+`wakeup/db.rs`).
- State: `cron.db` (`cron_jobs` + `cron_run_logs`), `sessions.db` (`subagent_runs`, `teams`/`team_*`), `background_jobs.db`, `wakeups.db` (agent self-scheduled `schedule_wakeup`). Config: `subagents.enabled`/`max_spawn_depth`, `team.*`.
- Grep: `category IN ('subagent','team','cron','wakeup')`.
- Gotcha: cron `status='disabled'` ← `consecutive_failures>=5`; non-null `running_at` after restart = orphan (`clear_all_running` should clear it); `cron_run_logs.status='timeout'` = exceeded 300s. `subagent_runs.child_agent_id` prefix `tool_job:` = async-tool injection (not a real subagent), `team:` = team member; stuck `Running` cleaned by `cleanup_orphan_runs`. `schedule_wakeup` fires go through `inject_and_run_parent` directly and **do NOT create a `subagent_runs` row** — they appear as a `ParentInjection`-source user message tagged `attachments_meta={"wakeup_trigger":{run_id}}`. Wakeup never fires → `wakeups.db` row exists (delivered rows are deleted) but no live timer (arming process exited; re-armed Primary-only on next restart) OR parent never went idle (injection `Abandoned`, left for replay); incognito wakeups are in-memory only (no row).

### File ops / Project / Canvas — `file-operations.md`, `project.md`, `canvas.md`

- Entry: `filesystem/workspace.rs` (`WorkspaceScope`), `filesystem/ops.rs`, `ha-server/routes/sessions.rs`, `project/files.rs`, `tools/canvas/`.
- State: `sessions.db` (projects/sessions), `memory.db` (project memory), `canvas/canvas.db`. Config: `filesystem.allow_remote_writes` (default false), `canvas.*`.
- Grep: `category='tool'` source `canvas` / file-op traces.
- Gotcha: HTTP `/api/fs/*` write 403 in server mode → `filesystem.allow_remote_writes` is off. Preview-by-path 403 → path neither referenced-by-tool-msg nor inside the session working dir (remote arbitrary-read guard). `for_path` scope is read-only — writes always rejected. Canvas snapshot/eval timeout on non-html content types is expected.

### Browser / macOS control — `browser.md`, `macos-control.md`

- Entry: `tools/browser/`, `browser/cdp_backend.rs`, `browser_state.rs`, `mac_control.rs`, `tools/mac_control.rs`, `src-tauri/src/macos_control.rs`.
- State: no SQLite — `browser-profiles/` + `browser/{managed-runner,user-attach,runtime}/`, `mac-control/snapshots/` + `mac-control/diagnostics/`. Config: `AppConfig.browser` (`launchCircuit`, `profiles[*]`).
- Grep: `category IN ('browser','mac_control')`.
- Gotcha: desktop-only — `mac_control` returns `supported=false` off macOS-Tauri and needs live TCC perms (Accessibility + Screen Recording); start with `action='status'`/`'permissions'`. Browser launch failures → `launch_circuit.rs` trip + 3-tier fallback. Missing `browser:frame`/`mac_control:frame` EventBus frames = capture failed. Stale `el_N`/`ref_id` only valid within their originating snapshot.

### Dashboard / Recap / Awareness — `dashboard.md`, `recap.md`, `behavior-awareness.md`

- Entry: `dashboard/`, `recap/`, `awareness/`, `agent/mod.rs`.
- State: `recap/recap.db` (rebuildable, keyed by `last_message_ts`), `sessions.db` (`learning_events`), `logs.db`, `cron.db`. Config: `awareness.enabled`/`mode` (hard-gate), `recap.analysisAgent`/`language`.
- Grep: `category IN ('awareness','recap')`.
- Gotcha: Dashboard stats must filter `is_cron=0 AND parent_session_id IS NULL`. Awareness short-circuits for incognito sessions (also excluded from stats); digest auto-clears after 3 consecutive side_query failures. `awareness.enabled=false` is a hard-gate that ignores per-session overrides.

### Ask-user / Prompt system / Image generation — `ask-user.md`, `prompt-system.md`, `image-generation.md`

- Entry: `tools/ask_user_question.rs`, `ask_user/questions.rs`, `channel/worker/ask_user.rs`, `system_prompt/build.rs`, `tools/image_generate/`.
- State: `sessions.db` (`ask_user_questions`). Config: `ask_user_question_timeout_enabled` (default false = wait forever) / `_secs`, `imageGenerate` (ordered `providers[]` = failover priority).
- Grep: `category='ask_user'`; image generation logs under `category='tool'` (source `image_generate`); IM ask_user uses `category='channel'` with an `ask_user:` prefix.
- Gotcha: pending ask-user is in-memory (`PENDING_ASK_USER_QUESTIONS` oneshot); startup flips all DB pending→answered, so a DB pending surviving a restart is an orphan (answering returns "No pending request"). Image gen failures return a transparent per-provider `failover_log`.

### Reliability / Security / Sandbox / Platform — `reliability.md`, `security.md`, `sandbox.md`, `platform.md`

- Entry: `guardian.rs`, `self_diagnosis.rs`, `crash_journal.rs`, `security/ssrf.rs`, `security/dangerous.rs`, `platform/mod.rs`, `docker/`.
- State: `crash_journal.json` (not SQLite), `backups/{ts}/` + `backups/autosave/`. Config: `guardian.enabled`, `dangerous_skip_all_approvals`, `ssrf{}`, `searxng_docker_use_proxy`.
- Grep: SearXNG / Docker sandbox → `category='sandbox'` (source `docker` / `searxng`); permission decisions → `category='permission'`. Crash recovery / Guardian / `self_diagnosis` have **no** `logs.db` category — read `crash_journal.json` directly (the primary signal).
- Gotcha: crash/restart loop → read `crash_journal.json` (`crashes[].signal`, last `diagnosis_result`); Guardian does not supervise `server`/`acp`. Dangerous-skip must be checked via `security::dangerous::is_dangerous_skip_active()` (CLI flag OR config), never the raw field. All outbound HTTP must pass `ssrf::check_url`.

### ACP / CLI / Slash commands / Config / Self-update — `acp.md`, `cli.md`, `slash-commands.md`, `config-system.md`, `self-update.md`

- Entry: `acp/agent.rs`, `slash_commands/handlers/`, `config/persistence.rs`, `updater/self_contained.rs`, `tools/app_update.rs`, `src-tauri/src/main.rs`.
- State: `sessions.db` (ACP shares SessionDB), `config.json`, `credentials/auth.json`. Config: `auto_update.*`, `permission.global_yolo`.
- Grep: `category IN ('self_update','acp','slash_cmd')`; config writes emit no log but fire the `config:changed` event.
- Gotcha: "UI save not taking effect until restart" class bugs ← a write that bypassed `mutate_config`. Binary swap must use `platform::atomic_replace_binary` + `--version` smoke test + auto-rollback. `MINISIGN_PUBKEY_BASE64` must equal `tauri.conf.json#plugins.updater.pubkey` or desktop startup panics. Headless auto-update loop only runs when `!is_desktop()`.

## Self-Study

1. Start with the architecture doc for the subsystem (see the Subsystem Reference).
2. Read the source that implements the documented contract.
3. Prefer public interfaces, command/route adapters, config structs, and tool schemas over isolated helper details.
4. Explain what happens in desktop and server modes when the behavior crosses transport boundaries.
5. Call out red lines: config mutation helpers, SSRF checks, provider CRUD helpers, failover policies, KB access gating, Plan Mode restrictions, and read-only DB rules.

## Feature or Improvement Issue

1. Restate the user request as a product outcome.
2. Identify affected workflows and likely modules.
3. Capture acceptance criteria.
4. Mention constraints from architecture docs.
5. Search existing issues before creating if duplicate checks are enabled.
6. Use `issue_report(action="draft")`, then `issue_report(action="create")` after the user requests submission or approves the draft.
