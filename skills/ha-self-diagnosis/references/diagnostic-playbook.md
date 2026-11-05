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

- Entry: kernel `ha-core/src/knowledge/access.rs` (access 唯一裁决) + `knowledge/registry.rs` (台账); ha-knowledge 的 `knowledge/service.rs` (owner plane), `knowledge/search.rs`, `tools/note.rs`（阶段 5 第六刀迁出；kernel 回调面 `knowledge_hooks.rs` 十槽）.
- State: `knowledge/index.db` (rebuildable cache); registry + attach bindings in `sessions.db` (`knowledge_bases` / `session_knowledge_bases` / `project_knowledge_bases`). Config: `knowledge_embedding`, `knowledge_search`, `knowledge_passive_recall`.
- Grep: `category='knowledge'` (sources: `service` / `index` / `reembed` / `embedding` / `maintenance::cycle`).
- Gotcha: access is deny-by-default via `effective_kb_access`; an agent that "can't see notes" usually has no attach — `SELECT kb_id,access FROM session_knowledge_bases WHERE session_id=?`. Empty search results → check `knowledge_embedding.enabled` (vector search degrades to FTS-only, never falls back to memory). Index drift → compare on-disk `.md` count vs `SELECT count(*) FROM note WHERE kb_id=?`.

### Design Space (设计空间) — `design-space.md`

- Entry: `design/service.rs` (owner plane), `design/renderer.rs` (self-contained HTML), `design/patch.rs` (oid writeback), `design/system.rs` (design systems), `design/critique.rs` (quality gate), `design/threads.rs` (per-project chat threads), `design/code_sync.rs` + `code_watcher.rs` (code→design 回灌), `design/mcp_provider.rs` (platform MCP provider — see `mcp-server.md`), `ha-design/src/tool_design/mod.rs` (agent tool). Frontend chat: `components/design/chat/{DesignChatPanel,useDesignChat}` (reuses main `useChatStream`).
- State: `design/design.db` (rebuildable metadata registry; tables incl. `design_systems`/`design_artifacts`/`design_comments`/`design_code_bindings`, plus `design_implement_receipts`/`design_code_links` for code→design 回灌 and `design_projects.last_opened_*` for MCP active-context); truth on disk under `design/{systems,projects}/` (`DESIGN.md`/`tokens.json`, `index.html`/`source/`/`versions/`/`oidmap.json`). Per-project AI chat threads live in **sessions.db** (`design_chat_threads`: `session_id`↔`project_id`, no cross-db FK) with `SessionKind::Design` (hidden from sidebar/`/sessions`/FTS); tool set narrowed by `ToolScope::Design`. Engineering axis (`token_export.rs`, Figma import in `extract.rs::from_figma`, `export_handoff`, `bind/sync_code_binding`) writes tokens to external code dirs — **owner-plane only**, HTTP gated by `filesystem.allowRemoteWrites`. Config: `design.*` (MEDIUM).
- Figma REST import: published styles need `library_content:read`, nodes/files need `file_content:read`, plus actual file/organization access. 401/403 show a safe permission hint; 429 accepts only bounded decimal `Retry-After` and `low`/`high` type. Unknown cooldown stays unknown. No automatic retry or error-body/upgrade-link/account-detail logging; GUI logs only its local diagnostic class and validated seconds. Confirm scopes without reading user credentials; synthetic fixtures do not prove live quota or import quality.
- Grep: `category='design'` (source: `service`; binding sync `source='binding_sync'`; design-chat thread mint `source='thread_mint'`); agent tool traces `category='tool'` source `design`.
- Gotcha: static artifacts are **self-contained HTML rendered directly** (no in-browser JSX compile) — a blank preview usually means a bad `resolveAssetUrl` rewrite or the static-serve gate rejecting the path, not a runtime crash. **code→design 回灌 not flagging stale** → no implement receipt (must go through 「实现到代码」) OR the session wrote files via `exec`/heredoc instead of `write`/`edit` (harvest reads tool metadata only); the `code_watcher` only watches parent dirs of already-harvested files (NonRecursive), so a never-harvested file won't trip it — the open-project deterministic `check_code_drift` is the fallback. **Platform MCP (`hope-agent mcp`)**: it's a separate short-lived process opening the same `design.db`; writes via MCP don't refresh the desktop GUI live (EventBus doesn't cross processes — reopen the artifact); `design_edit_element` requires `expectedBodyHash` (cross-process has no shared `artifact_lock`); disabled when `design.enabled=false` (tools/list empty + call rejected). Design chat editing the wrong / a new draft project instead of the open one → the `kind=design` session isn't anchored in `design_chat_threads` (project resolution falls back to `list_projects_by_session`); check the `chat` command promoted it (`tool_scope=design` + `designProjectId` on the auto-create send). **`component` kind is backend-compiled (oxc)** → a compile failure renders a static error page (not blank/panic); the shipped `index.html` is the compiled output, so all export paths compile via `render_clean`. Visual edits failing → `oidmap.json` stale vs current `body.html` (BLAKE3 mismatch → `patch_element` rejects as stale-write, by design; reload to resync). Tokens not applying → the artifact's `system_id` / project `default_system_id` is null or points at a missing `systems/{id}/tokens.json`. Code-binding sync writing nothing / "no tokens to sync" → the bound system's `tokens.json` is empty or unreadable (sync is fail-closed to avoid overwriting real project files with empty skeletons).

### Desktop Pet (桌面宠物) — `pet.md`

- Entry: `ha-pet/src/` (library/import, activity projection; kernel 面 `ha-core/src/pet.rs` ChatUiSurface + activity 通知、`session/pet_activity.rs` 行查询), `src-tauri/src/commands/pet.rs` + `src-tauri/src/pet_window.rs` (desktop shell/window geometry), `ha-server/src/routes/pet.rs` (HTTP adapter), `src/PetWindow.tsx` (renderer), `src/components/settings/PetSettingsPanel.tsx` (owner UI).
- State: `config.json` (`pet.enabled` / `pet.selectedPetRef`); `sessions.db` (`chat_turns.ui_surface`, `sessions.last_read_message_id`, pending approval/ask state); custom packages under `pets/` and PetWindow anchor state in `pet-window-state.json` under the app data directory. Import previews are bounded in-process cache entries (restart discards them); uploaded sources remain governed by the shared file-upload lease.
- Grep: `category='pet'` (import validation/commit, store recovery, Tauri preview argument rejection); window lifecycle fallbacks also appear under `category='app'` with a Pet source.
- Gotcha: no bubble for a conversation → inspect its newest `chat_turns.ui_surface`; only the explicit first-party surface allowlist is projected, and a newer non-UI turn intentionally removes eligibility. A completion bubble vanishing after a failed click → PetWindow must not call `mark_session_read_cmd` during navigation; only the destination transcript advances the watermark after a successful render. White rectangle behind the sprite → the Pet boot path lost its static `bg-transparent` utility. Import preview errors that occur before a Tauri handler are recorded by the frontend rejection bridge, so grep `category='pet'` with `source='PetSettingsPanel::previewSources'` rather than expecting command-body logs.

### Built-in User Manual / Help Center — `help-center.md`

- Entry: `manual/mod.rs` (API + commands), `manual/embed.rs` (rust-embed of `docs/user-guide`), `manual/unpack.rs` (stable mirror), `skills/ha-manual/SKILL.md` (agent path), `src/components/help/` (GUI).
- State: mirror at `<data-dir>/manual/{zh,en}/NN.md` + sibling marker `.manual-synced` (fingerprint; safe to delete — rebuilt on next use). No DB, no config.
- Grep: `category='manual'` (sources: `mirror` / `parse`).
- Gotcha: agent "manual not found" → mirror not materialized; check the marker exists and `manual/zh/index.md` is present, then grep `category='manual'` for the mirror failure. Under Docker the data dir is `$HA_DATA_DIR` (`/data`), not `~/.hope-agent` — the skill resolves it dynamically, hand-written paths often don't. Empty manual in a self-built binary → the build tree lacked `docs/user-guide` at compile time (Dockerfile must COPY it before `cargo build`; the `manual::embed` unit test is the gate). Anchor jumps landing nowhere in the Help window → slug drift between Rust `github_slug` and frontend `manualSlug.ts` (both carry the same ground-truth test pairs).

### Hooks — `hooks.md`

- Entry: `hooks/mod.rs` (`HookDispatcher::dispatch`), `hooks/scopes.rs`, `hooks/runner/`, `hooks/parse.rs`; block points `agent/preflight.rs` (UserPromptSubmit) and `tools/execution.rs::fire_pre_tool_use_hook` (PreToolUse).
- State: `AppConfig.hooks`, `disable_all_hooks`（总开关）, `hook_workspace_trusts`（canonical 路径 + project/local 内容哈希；逐工作区授权）。旧 `hooks_allow_project_scope` 只为配置兼容保留，执行层忽略。
- Grep: `category='hooks'` (source `dispatch` logs event/handler-count/decision/duration).
- Gotcha: project/local hooks not firing → 工作区尚未在 Settings → Hooks 授权，或 Hook 文件在授权后发生变化；需复核内容后重新保存信任。路径别名、symlink 与移动后的目录不会继承信任。Matcher normalizes Claude aliases (`Bash`→`exec`) but the payload `.tool_name` stays the internal name, so a script matching `.tool_name=="Bash"` never fires. Only `UserPromptSubmit`/`PreToolUse`/`PreCompact` actually block; the other 21 events are observation-only.

### Chat Engine / Session / Compaction / Memory — `chat-engine.md`, `session.md`, `context-compact.md`, `memory.md`

- 记忆写作风格未生效 → 核查 `memory.promptPreferences.{extraction,profile,dreaming}` 及 Agent 各环节覆盖（缺失/null 继承，显式 default 恢复系统风格）；画像只在手动模型重写中生效，自动画像零 LLM，共享 Dreaming 日记只用全局偏好。入口：`memory/prompt_preferences.rs`、`ha-memory/src/extract.rs`、`dreaming_profile.rs`、`dreaming_narrative.rs`。只核查阶段与风格，不记录补充要求正文。

- Entry: `chat_engine/engine.rs`, `chat_engine/stop.rs`, `chat_engine/finalize/`, `session/db.rs`, `session/autonomy_pause.rs`, `tools/session_continue.rs`, `context_compact/compact.rs`, `memory/sqlite/`, `memory_extract.rs`.
- State: `sessions.db` (`context_json` snapshot, `messages.stream_status`, `messages.is_side_snapshot`, `chat_turns`, `session_autonomy_pauses`, session-free `runtime_control_epochs`), `memory.db`. Config: `compact.*` (`cacheTtlSecs=300`, override at usage ≥0.95), `memoryExtract.*`, `memory_embedding`.
- Grep: `category IN ('chat_engine','memory')`; compaction logs under `category='context'` (source `compact`) plus `category='agent'` (`reactive_microcompact`).
- 摘要正文为空或推理档位失败 → 检查 `compact.summaryReasoningEffort`（null 保持默认）；仅摘要请求应用显式覆盖，不改变主对话推理或输出上限。不支持的档位/关闭参数以 request-contract 终态停止且保留原历史；先核对摘要专用模型、协议和错误类型，不自动切换模型或扩预算，第三方网关需单独验收。
- Codex 本地记录导入：入口 `session/codex_import.rs`，台账 `sessions.db.session_import_sources`；`category='session' AND source='codex_import'` 只记录汇总计数。`SELECT source_id,session_id FROM session_import_sources WHERE provider='codex'` 可检查去重关系；这些会话故意只读，`TurnKernel` 和消息插入触发器均拒绝续写。检查失败时不要读取或贴出 JSONL 正文、凭据与工作目录。
- 主对话关闭镜像后侧聊无法自动打开 → 检查浏览器与 Mac 是否都经 `useMirrorPanelSessionScope` 绑定当前对话面，显示与关闭状态不能再由父会话工作台缓存覆盖。
- 侧聊宠物提示点击无效 → 检查所属主会话是否仍符合 `regular_session_scope_sql`、侧聊自身是否未归档；归档或恢复应触发 `pet:activity_changed`，不能为不可打开的主会话继续投影侧聊活动。
- 侧聊记忆重复 → 阈值/空闲提取只读 `load_side_chat_memory_history` 的非快照正文；压缩前提取保留丢弃片段并排除 `_ha_side_snapshot`，不能读保留区。`sessions.side_snapshot_context_version=0` 的旧侧聊应跳过压缩前提取。工具结果回读缺引用 → 检查侧聊 `session_result_refs` 的快照消息重映射及稳定上下文范围，不能直接放宽授权。终态角标被后续命令重新点亮 → 检查 `chat_turns.terminal_message_id`，该值由终结事务固定为回答或通知标识，不能在查询时随最新消息移动。
- Gotcha: Anthropic 400 on tool_use/tool_result → API-round (`_oc_round`) boundary broke; `prepare_messages_for_api()` strips metadata. Leftover partials → `SELECT id,stream_status FROM messages WHERE stream_status IN ('streaming','orphaned')`; stale turns → `SELECT status,interrupt_reason FROM chat_turns WHERE status IN ('running','cancelling')`. 侧聊创建后统计突然翻倍 → 检查复制历史是否为 `messages.is_side_snapshot=1`；这些行只提供上下文、Dashboard 必须排除，侧聊创建后的新消息保持 `0` 并正常计数。Stop 后重启仍在跑 → `SELECT id,session_id,created_at,resumed_at,global_stop_epoch FROM session_autonomy_pauses WHERE resumed_at IS NULL`；Continue 必须使用当前 `<session-paused>` 的 exact `pause_id`，model tool 还须来自 admission epoch 不早于该 Stop 的 foreground turn。Global Stop 漏掉 Server / ACP → `SELECT run_id,session_id,source,status FROM chat_stream_runs WHERE status='running' AND source IN ('desktop','http','channel','acp')` 应全部对应到 pause root；incognito 不会出现于此表，改查 `SELECT epoch,updated_at FROM runtime_control_epochs WHERE key='global_stop'` 是否推进，各进程 250ms watcher 只按 immutable run id signal 自己持有的 foreground token。Stop receipt 偶发不落 → 检查是否误用 deferred transaction 导致 WAL `SQLITE_BUSY_SNAPSHOT`。Stop 返回后某个 child / parent injection 仍跑 → 对照 `subagent_runs.runner_owner` 与本进程 owner，并查 `chat:stop_handoff`；禁止 Secondary 伪造 remote terminal，且已 resumed 的 pause row 仍按 epoch/run id 取消旧 generation。Continue 返回成功但 workflow / wakeup 不动 → `SELECT id,session_id,resume_requested_at,resume_replayed_at,resume_replay_error FROM session_autonomy_pauses WHERE resume_requested_at IS NOT NULL ORDER BY resume_requested_at DESC`；`resume_replayed_at IS NULL` 表示 Secondary→Primary durable handoff 尚待 2 秒 sweep 认领，持续不收敛再检查 Primary ownership / `chat:continue_handoff` 日志。`recall_memory` excludes Project scope by design. `memory_embedding.enabled=false` → recall is FTS5-only.

- OpenViking 列表：顶层 `has_more=true`、畸形标志/数组、超过5000原始节点或旧版无标志且达到上限时明确失败；不读内容、不推进版本、不启动新导出，已提交导出仍可先对账。不删除账本绕过，不把可变列表的 offset 分页当成可靠续扫；真实版本/服务验收另列。
- OpenViking 同步：`{provider}.sync.json` 的 `openVikingPendingExports` 是未决写入围栏，只存记忆摘要、task ID 和凭据指纹。`accepted` 不是成功；缺少明确零跳过证明、已跳过写入、失败/404/未知 task 或身份改变先在原服务对账，不删除账本或自动重发。删除连接也保留未决/异常同步账本，重建同ID不能绕过；孤儿清理异常不应中止其它密钥文件删除。不要输出记忆正文、API key 或整份凭据文件。

### Provider / Failover / Side-query / Local LLM — `provider-system.md`, `failover.md`, `side-query.md`, `local-model-loading.md`

- Entry: `failover/executor.rs` (`execute_with_failover`), `chat_engine/engine.rs`, `agent/side_query.rs`, `provider/crud.rs`, ha-local-llm 的 `local_llm/management.rs`（阶段 5 自 ha-core 迁出；任务台账 `local_model_jobs.rs` 仍在 ha-core）.
- State: `config.json` (`providers` / `active_model` / `fallback_models`); `sessions.db` (`provider_id` / `model_id` pin, `context_json`); `local_model_jobs.db`, `local_llm_library_cache.db`; `credentials/auth.json` (Codex OAuth).
- Grep: `source='failover'` (profile_rotation / codex_auth_expired / model_fallback); `category IN ('local_llm','local_model_jobs')`.
- 模型适配诊断：`category='provider' AND source IN ('retry_deferred','anthropic_thinking_binding')` 分别表示服务端等待下限超预算、Claude 前缀或模型绑定失配。`misalignment_policy_violation` 属于停止工作流的终态，不要换密钥或模型重试；普通 403 仍按鉴权问题处理。Astra 有工具须用官方 Responses；Claude 原始签名与遮蔽块不得手工删改。Anthropic 多工作区密钥检查设置页对应鉴权档案的 `anthropicWorkspaceId`，缺失值须发送前拒绝，冷却和轮换不能跨已声明工作区；诊断只记录是否已绑定，禁止打印请求头或密钥。
- Gotcha: model precedence is `plan_model > model_override > session pin > agent.primary > active_model`. Codex is force-excluded from profile rotation. OpenAIResponses must use `store:false` and never replay reasoning items (`rs_*` 404s) — check `context_json` for leaked `type:reasoning` items.
- Responses / Codex 工具续轮出现 `Expected an ID that begins with 'fc'` 时，核对 `function_call.id` 与 `call_id` 是否被混用：原始服务端条目保留独立标识；合成或中断重建只填 `call_id`，省略可选 `id`。旧历史中 `id == call_id` 且不以 `fc` 开头的值由标准化修复，不要手改调用或结果的配对标识，也不要在诊断中输出会话正文或凭据。
- **Vision bridge (issue #434)** → `category='agent'` source `vision_bridge`; usage rows are `model_usage_events.kind='vision'` (distinct from `side_query`). Entry `agent/vision_bridge.rs`, gated in `streaming_loop.rs` round head. Only engages when the **main** model is text-only (`model_supports_vision==false`) AND `config.function_models.vision` is set — otherwise off (image dropped with `[image omitted]`, the pre-existing degrade). "Sent an image to DeepSeek but it says it can't see it" → check (a) a vision model is configured in Settings → global models, (b) that model is genuinely vision-capable (`input_types` includes `image`), (c) `logs.db` `source='vision_bridge'` for a transcription failure/timeout (falls back to placeholder, never hard-fails the turn). Rewrites operate on the **ephemeral** `api_messages` copy only — `conversation_history` / `context_json` keep the raw images (reversible). Transcriptions are memoized process-wide by `(image identity, vision model)`; a restart clears the cache (re-transcribes once). Incognito runs the bridge but skips the ledger (no `kind='vision'` row).

### Speech-to-Text (STT) — `stt.md`

- Entry: kernel `ha-core/src/stt/` (`engine.rs` 链解析 + failover trampoline, `crud.rs` single write entry, `local.rs` 4 local backends); 执行机器 `ha-media/src/stt/` (`engine.rs` `failover_transcribe_batch` 真实现, `session.rs` `SttSessionManager`, `providers/` wire kinds). Owner surface: `src-tauri/src/commands/stt.rs` (20 cmds), `ha-server/src/routes/stt.rs` (17 routes, masked).
- State: `config.json` (`stt.providers` / `stt.active_model` / `stt.fallback_models` / `stt.im_fallback_model`) — **separate from LLM `providers`**; sessions are in-memory (**no DB**); IM toggle `channels.accounts[*].settings.autoTranscribeVoice`.
- Catalog: new xAI batch preset `grok-voice-transcribe-2.0`; AssemblyAI WS `universal-3-6-pro` first. Saved/custom IDs stay intact; xAI word timing is not segment alignment, and AssemblyAI remains excluded from batch/IM fallback. Loopback/URL fixtures do not certify live recognition or billing.
- Grep: `category='stt'` (session GC eviction). EventBus `stt:transcript_partial` / `stt:transcript_final` / `stt:session_error`.
- Gotcha: no active model → `SttError::NoActiveModel` (fail-closed, never "any model"). Fallback / IM chains reject WS-only kinds via `check_batch_capable` (`supports_batch()`); desktop `active_model` may use WS (streaming). Size caps `MAX_BATCH_AUDIO_BYTES`=25 MiB (Tauri + HTTP + provider load) / chunk 1 MiB. WS providers are SSRF-checked through an http(s) twin (`ws_to_https_twin`). Provider list + keys are **owner-GUI-only** (not `update_settings`); only the `im_auto_transcribe` toggle is model-writable. Idle sessions GC'd after 300s. STT config is global — not incognito-scoped.


### Agent config / Onboarding / OpenClaw import / Main-LLM OAuth — `agent-config.md`, `onboarding.md`, `openclaw-import.md`, `llm-oauth.md`

- Entry: `agent_config.rs` (`AgentConfig` + runtime `AgentDefinition` + `effective_memory_budget`), `agent_loader.rs` (`load_agent`/`list_agents`/`ensure_default_agent`/`DEFAULT_AGENT_ID`/`is_main_agent`), `agent/resolver.rs` (`resolve_default_agent_id_full` — 7-level chain + `AgentSource`), `agent/migration.rs` (legacy `default`→`ha-main`). Onboarding: `onboarding/{state,presets,apply}.rs`. OpenClaw import: `openclaw_import/{mod,agents,providers,memory,paths}.rs`. Main-LLM (Codex) OAuth: `oauth.rs`. Owner surface: `ha-server/routes/agents.rs` + `src-tauri/src/commands/`.
- State: `agents/{id}/agent.json` (+ `agent`/`persona`/`tools`/`memory`/`agents`/`identity`/`soul`.md) — **the directory name IS the agent id; `AgentConfig` has no `id` field**; canonical Core Memory lives at `~/.hope-agent/memory/MEMORY.md` and `~/.hope-agent/agents/{id}/memory/MEMORY.md` (legacy lowercase files are compatibility mirrors only), plus `{id}-home/`, `plans/{agent}/`, migration sentinel `.agent-id-renamed`. Config: `AppConfig.default_agent_id` (chain L7) + `agent_order`. OAuth creds: `credentials/auth.json`. Onboarding `apply` writes through provider CRUD (`provider/crud.rs`) + `mutate_config` (never raw).
- Grep: `category='auth'` (OAuth login/refresh/logout), `category='agent'` source `migration` (the `default`→`ha-main` rename). Agent CRUD / onboarding / openclaw-import have **no dedicated log category** — trace via the `agents:changed` / `config:changed` EventBus events + the on-disk state.
- Gotcha: wrong agent for a new session → walk `resolve_default_agent_id_full` 7 levels (Explicit → project → topic → group → tg_channel → channel_account → `AppConfig.default_agent_id` → hardcoded `"ha-main"`, first non-empty wins); `_with_source` tags which level hit (shown in `/status`). **`is_main_agent` ("ha-main") is orthogonal to `default_agent_id`** — changing the global default does NOT change which agent is "main" (main-ness only drives tool-tier enrichment `default_for_main`). `AgentConfig` is **all-`serde default`** — a missing/corrupt `agent.json` silently falls back to `AgentConfig::default()` (load never fails; `list_agents` skips unparseable dirs). Migration is one-shot: sentinel `.agent-id-renamed` is written **only after the disk dir rename completes cleanly**; if both `agents/default/` and `agents/ha-main/` exist it **aborts whole** (no sentinel, retries next boot) — and `init_runtime` MUST precede `ensure_default_agent()` or the latter pre-creates an empty `agents/ha-main/` that makes migration mis-abort. OAuth: only the Codex provider uses it; `clear_token` deletes `auth.json` + fires `session_end("logout")` but the **Codex provider-row deletion lives in the 3 logout shells, not `clear_token`**; concurrent refreshes de-dup; tokens **never** logged (red-line). OpenClaw import is 3-phase (scan → preview → selective import); providers de-dup first-imported-wins, imported `MEMORY.md` merges into Hope Agent's canonical `MEMORY.md`, while only SQLite chunk rows insert as `NewMemory(source="openclaw-db-import")`.

### Tools / Permissions / MCP / IM Channel / Skills / Logging / Background Jobs — `tool-system.md`, `permission-system.md`, `mcp.md`, `im-channel.md`, `skill-system.md`, `logging.md`, `background-jobs.md`

- Skill installation: `skills/ha-skill-installer/scripts/install_skill.py` prepares a temporary snapshot, verifies its digest, and atomically publishes without replacement. The installed `.hope-skill-install.json` records source and file hashes. Use `skill({name, action:"inspect"})` to refresh the catalog and compare `baseDir` with the intended destination; inspect never activates the skill. Trace `category='skills'`, `source='inspect'`. Missing prerequisites or a different winning source are distinct from failed file installation.

- Entry: `tools/dispatch.rs` (visibility), `tools/execution.rs`, `tool_defs/` (kernel-shared contract: `TOOL_*` names, `ToolDefinition`/tier, `ToolExecContext`, scope predicates), `permission/engine.rs` (`resolve_async`), `ha-mcp/src/invoke.rs` (runtime; kernel facade `ha-core/src/mcp/`), ha-channel 的 `channel/worker/dispatcher.rs`, ha-skills 的 `skills/discovery.rs`·`skills/author.rs`·`tools/skill/`（阶段 5 第七刀迁出；kernel 回调面 `skills_hooks.rs` 九槽，契约 `skills/types.rs` + 台账 `skills/activation.rs` 仍在 ha-core）, `logging/db.rs`, `async_jobs/manager.rs` (`JobManager`).
- 企业微信首版入口 `ha-channel/src/channel/wecom/{mod,connection}.rs`，配置 `channels.accounts[*]` 的 `channelId=wecom`。健康页分开看 worker 存活与 probe_ok；poll 仅读缓存。鉴权拒绝后同凭据不自动重连，被其它客户端替换后也不竞争重连；同一 Bot 更改 Secret 时先停止本机运行账户，再显式校验新值；校验不会为不同凭据的同 Bot 建第二条连接。被替换时先停止其它客户端再重启。`category='channel' AND source='wecom'` 拒绝日志只含脱敏 ID、每账号最多30秒一条，可核对 `single:<userid>` / `group:<chatid>` 与 allowlist；不要打印正文、Secret、完整 credentials/frame。无原始锚点、超过10分钟/512项容量/重连后的锚点、已用锚点、媒体/主动投递都拒绝；ACK 未知不可重发。新群 requireMention=false 需 owner 明确加入，已有 mention-only 不迁移放行。审批/提问从应用界面处理，未作真实租户验收。
- State: `sessions.db` (`channel_conversations`, `sessions.permission_mode`, `learning_events`), `background_jobs.db`, `permission/*.json`, `credentials/mcp/{id}.json`. Config: `mcp_global.enabled`, `permission.global_yolo`, `deferredTools.enabled`.
- Grep: `category IN ('tool','mcp','channel','skills','permission','async_jobs')` (permission decisions log under `permission`; background-job lifecycle logs under `async_jobs`).
- 免 Key 搜索：`tools/web_search/keyless.rs` 先尝试 Brave 网页、再回退 360 网页搜索，日志仍用 `category='tool'` / `source='web_search'`，用量操作为 `keyless_search`。403 / 429 后该引擎冷却 60 秒；上游限流、页面变更和确实没有结果须区分。360 网页入口不支持时效过滤，指定 `freshness` 时跳过；不能把免 Key 当作始终可用的保证。
- MCP successful tool data: `ha-mcp/src/invoke.rs::normalize_result_for` preserves `structuredContent` (including explicit null) after text/media fallbacks; absent means legacy output. `_meta` stays out of model text. Error results remain `ToolFailed` with existing text-only diagnostics; structured error payloads are not copied into Learning events. This is data forwarding, not output-schema validation or full protocol acceptance. MCP hooks use caller-selected protocol output: recognized, typed hook JSON is a candidate, including empty nested and nullable protocol fields; metadata/malformed values fall back to legacy text. For two protocol payloads, event-specific stricter decisions and continue:false take precedence; permission_allow requires agreement from both protocol channels, while contexts and legacy rewrites are retained. Check `ha-mcp/tests/mcp_hook_results.rs` for the in-memory wire-to-hook parser regression; conversation suffixes must never reach HookOutput.
- Gotcha: tool not callable → trace `resolve_tool_fate` (visibility) then `resolve_async` (Decision); check `sessions.permission_mode` (`default|smart|yolo`) and `permission.global_yolo`. Hiding a tool is never a security boundary. MCP handshake 401/403 → `ServerState::NeedsAuth` (no retry loop). IM not replying / wrong session → `SELECT * FROM channel_conversations WHERE session_id=?` (1:1 attach both ways). **Background jobs (R1 naming split)**: the DB is `background_jobs.db` (table `background_jobs`, `JobKind` tool/subagent(R6)/group(R5) — all three populated) and the single code entry is `async_jobs::JobManager`, but the Rust **module + log category stay `async_jobs`** (lineage) — so grep job lifecycle under `category='async_jobs'`, not `background_jobs`. Stale-schema probe is `SELECT group_id` (newest col); on upgrade a pre-R5 table + the legacy `async_jobs.db` / `async_jobs/` are best-effort dropped/deleted (pure rebuildable cache, no migration). EventBus lifecycle events use the `job:*` prefix (R3, was `async_tool_job:*`) — distinct from the `async_jobs` LOG category; `subagent` kind keeps `subagent:*` events (no double-emit), so the R4 panel merges both streams.
- 飞书获取令牌失败：`feishu/auth.rs` 仅保留 HTTP 状态或业务代码；JSON 类型错误不带正文，失败不入缓存。按该代码定位，不索取或打印 App Secret / tenant_access_token；本地至少4KiB双账号夹具不等同真实租户或其它 API 错误路径验收。
- Google Chat 预览格式变化：创建与编辑均显式选择 `markupSyntax`，PATCH 的 `updateMask` 仅为 `text`；不要将语法加入 mask 或修改云端默认。编辑按账户、space、message 的进程内有界记录固定创建模式；记录缺失、重启或淘汰会拒绝编辑，不猜语法、不重新发送。账号切换、4xx/429/超时仅有回环证据，真实 Workspace 展示/mention 通知仍需独立授权验收。
- Signal 接收乱码/停止卡住：检查 `channel` / `signal-sse` 与 `signal/sse.rs`；按完整UTF-8行解析，CR/LF/CRLF及多行data保留。非法UTF-8或未完成行+事件超过1MiB时仅报固定错误类别并退避重连，不输出原消息；响应头和入站队列等待也应可取消。未启用Last-Event-ID重放，断连期间消息与真实daemon互操作另行验收，不删除队列或自动重放工具。
- iMessage 编辑/撤回不触发回复：`ha-channel/src/channel/imessage/client.rs` 在入站和 catch-up 的游标/去重前分类；imsg v0.19 新消息省略 `type`，`edited` / `unsent` 及任何显式未知或畸形类型均丢弃。它们不推进插入游标、不占原消息 GUID，也不更新/删除会话历史。ROWID 恢复不能证明历史变更完整；不要为排查自动扫描 Messages 数据库或打印原始载荷。真实设备与带外变更同步仍未验收。


### Project operations automations — `automation-operations.md`

- Entry: `scripts/automation/control.py`; installed into the explicitly authorized `~/.codex/automation-runtime/hope-agent` directory alongside the existing prepare helper. This is Codex project maintenance, not product Cron or an alternative Agent loop.
- State: `policy.json` / `tasks.json`; `control-state/{health,writer-lease}.json` and scoped observation hashes; immutable recovery manifests and receipts under `recovery/`. Codex scheduling DBs are read-only inputs. Inbox `PENDING_REVIEW` is not run success: inspect native thread terminal state.
- Gotcha: `writer_busy` means inspect the exact owner and its pending PR before release; never steal on timeout. Missing or invalid archive input is an evidence gap; legacy LATEST indexes must resolve to validated immutable files. Permission failure stays closed. Recovery covers project operations assets only, never user data or credentials; local hash verification is not offsite acceptance.

### Subagent / Team / Cron — `subagent.md`, `agent-team.md`, `cron.md`

- Entry: `subagent/injection.rs`, `team/coordinator.rs`, ha-cron 的 `cron/scheduler.rs`·`cron/executor.rs`（阶段 5 迁出；台账 `cron/db.rs` 仍在 ha-core）, `session/subagent_db.rs`, `wakeup/mod.rs` (+`wakeup/db.rs`).
- State: `cron.db` (`cron_jobs` + `cron_run_logs` + `cron_meta`), `sessions.db` (`subagent_runs`, `subagent_result_deliveries`, `teams`/`team_*`), `background_jobs.db`, `wakeups.db` (agent self-scheduled `schedule_wakeup`). Config: `subagents.enabled`/`max_spawn_depth`/`providerRetryAttempts`/`providerRetryBackoffSecs`, `team.*`, `cron.{maxConcurrent,jobTimeoutSecs,atGraceSecs}`. Cron-panel conversation surface: `cron_run_timeline` / `cron_unread_total` / `cron_mark_all_read` (Tauri + HTTP `/api/cron/{timeline,unread,read-all}`) + event `cron:unread_changed`; the cross-job timeline is assembled in `cron::timeline` (joins `cron.db` run logs with `sessions.db` title/unread in Rust — the two DBs can't be SQL-joined).
- Grep: `category IN ('subagent','team','cron','wakeup')`.
- Delivery retry: `SELECT run_id,state,attempt_count,requested_at,suppress_reason,last_error FROM subagent_result_deliveries ORDER BY requested_at`; due `pending` rows由 Primary 的 5s durable replay sweep 重新 claim，未来时间行等待退避到期属于正常；`suppress_reason='provider_retries_exhausted'` 会保留为 parent 的 `<runtime-recovery>` 待检查，不再自动重投。
- Gotcha: cron `status='disabled'` ← `consecutive_failures>=max_failures` (default 5, **genuine run failures only** — infra errors like session-create-fail route through `reschedule_without_failure` with `count_toward_disable=false` and do NOT bump the counter, so transient DB blips can't disable a healthy job); non-null `running_at` after restart = orphan (`clear_all_running` clears it; in-process panics covered by RAII `RunningMarkerGuard`). `cron_run_logs.status` is **11-valued** — the pre-model states `preparing` / `queued` (a `SessionTurn` staged into the target chat's queue; `list_stale_preparing_session_turn_runs` repairs a crash inside that window), the settlement states `cancelling` / `completing`, plus `classify_cron_terminal`'s terminals and the no-session infra path: `running` (in-flight / crash breadcrumb — `recover_orphaned_runs` finalizes stale ones to `error` on startup), `success`, `empty` (non-cancelled zero-output `Ok` — delivery skipped / no empty message sent, schedule still advances; **recurring `empty` does NOT pop a desktop toast** — `notify_empty = notify_on_complete && At`, only one-shot `At` notifies), `cancelled` (cancel interrupt — no empty delivery, schedule NOT advanced; one-shot `At` terminalized to `completed`), `timeout` (exceeded the per-run budget `CronConfig.job_timeout_secs`, default 600, clamp `[30,7200]`, per-job `CronJob.job_timeout_secs` override — **not a fixed 300s anymore**), `error`, `no_session` (infra failure: session creation failed / turn never ran — counts as failure in the dashboard denylist but routed via `reschedule_without_failure`, so it does NOT bump `consecutive_failures` / auto-disable). Concurrency: `CronConfig.max_concurrent` (default 5, `0`=unlimited) caps in-flight runs; the scheduler grabs a slot **before** claiming (slot-before-claim), so jobs beyond the cap keep `next_run_at` and retry on the next 15s tick (not skipped/lost) — a job not firing under a full pool is starvation, check `count_running()` for leaked `running_at` markers. A scheduled `success`/`empty` run releases its slot (`clear_running`) **before** the IM delivery (so a hung delivery target can't pin a cap slot); run-now keeps its slot through delivery. **run-now (`start_cron_run_now`, all 3 entry points: Tauri / HTTP / `manage_cron`) is Primary-only** — a non-Primary instance refuses it (returns an error, not a fake success) and `mark_missed_at_jobs` now runs every tick before dispatch (not just at startup). `cron_meta` KV holds `scheduler_heartbeat` (stale ≥300s on startup → `app_warn` "scheduler was offline ~Ns"; Primary-only, crash recovered via catch-up on restart) + one-time `tz_backfill_done` sentinel (gates host-timezone backfill of null-tz `Cron` rows; absent = backfill will re-run, host-undetectable = sentinel not written so it retries). `subagent_runs.child_agent_id` prefix `tool_job:` = async-tool injection (not a real subagent), `team:` = team member; stuck `Running` cleaned by `cleanup_orphan_runs`. **R6 subagent projection**: a user-delegated background subagent run ALSO gets a one-way `background_jobs` row (`kind='subagent'`, `subagent_run_id` FK) so it shows in `job_status list/cancel` — `subagent_runs` is the truth source (result/error live there; the projection has none). Projection status drifting from `subagent_runs.status`? sync is hooked in `update_subagent_status` (the choke point) — `cleanup_orphan_subagent_runs` performs a recovery transaction, then explicitly syncs the Background Job projection and Workflow notification; after restart both the run and projection read `interrupted` with `terminal_reason='process_interrupted'`, and ordinary-parent pending delivery is replayed. Cancelling a `kind=subagent` job routes to `subagent::request_cancel_run`, NOT the tool-job cancel path. Internal plan/team/hook subagents (`skip_parent_injection=true`) and incognito-parent runs are NOT projected. **R7.2 reject→queue**: a subagent that hits the per-session concurrency limit (`max_concurrent_for_agent`, default 8) now PARKS as `subagent_runs.status='queued'` (projection `kind=subagent` status `queued`) instead of erroring — held in the in-memory `subagent::queue` (live `SpawnParams`, lost on restart). The per-process `run_subagent_scheduler` promotes it (`queued→spawning`) when a running child settles (woken from the `update_subagent_status` terminal choke point) + a 5s fallback tick. Stuck `queued`? check the session is actually under its limit (`count_active_subagent_runs` counts only `spawning`/`running` — `queued` is excluded by design so it can't deadlock its own promotion) and that a sibling actually reached terminal to fire the wake; restart sweeps `queued` rows to `Interrupted(process_interrupted)` via `cleanup_orphan_subagent_runs`, syncs their projections, and replays eligible ordinary-parent delivery. Structural limits (depth/batch/agent-exists) still hard-reject, never queue. A `kind=subagent` projection sitting at `awaiting_approval`? that label is set by the **R8-follow-up `async_jobs::approval_projection_watcher`** — it subscribes to `approval_required` / `approval:resolved`, maps the approval's child session → run via `SessionDB::find_active_run_by_child_session` → projection, and flips `running⇄awaiting_approval` (pure label, never gates execution; the inner approval still block-and-waits). Stuck at `awaiting_approval` after the approval was answered? resume only fires on the `approval:resolved` event — check the EventBus delivered it (watcher lag logs under `category='async_jobs'`, source `approval_projection`); `subagent_runs.status` is the truth (the label is a view). **R5 Group fan-out**: `batch_spawn` creates a `kind=group` coordinator row + N `kind=subagent` children sharing `group_id`; the children **suppress their individual injection** and the group fires ONE merged `<subagent-result>` when all children settle (single-winner CAS `claim_group_completion`). Batch ran but no merged notification arrived? check the group row: `args_json={"sealed":true}` must be set (set after the spawn loop — if a malformed task `?`-returned before R5's pre-validation fix the group would never seal) AND all children terminal; the merged message reuses the `subagent_result` meta (agent_id=`batch`, run_id=`<group_id>`). Group rows are `injected=1` (own merged injection, never the tool-replay path); on restart, child cleanup first syncs each orphaned attempt to `interrupted`, so a sealed group with complete child projections still converges through the single-winner join and emits one merged result. Only a residual group that cannot converge there is later marked `interrupted` without a fabricated merge (child truth remains in `subagent_runs`). Cancelling a `kind=group` marks the group terminal FIRST then cancels children (so the join CAS loses — no injection for a cancelled batch). `schedule_wakeup` fires go through `inject_and_run_parent` directly and **do NOT create a `subagent_runs` row** — they appear as a `ParentInjection`-source user message tagged `attachments_meta={"wakeup_trigger":{run_id}}`. Wakeup never fires → `wakeups.db` row exists (delivered rows are deleted) but no live timer (arming process exited; re-armed Primary-only on next restart) OR parent never went idle (injection `Abandoned`, left for replay); incognito wakeups are in-memory only (no row). **A scheduled run is now an ordinary chat** (`is_cron=0`, `origin_json.kind='cron'`) and DOES appear in the sidebar, search, pins and regular unread — only pre-`AgentTurn` legacy rows still carry `is_cron=1` (`mark_session_cron` survives solely on the legacy path in `cron/timeline.rs`). "My cron ran but its conversation isn't in the sidebar" is now a real bug (check `origin_json`), the inverse of the old answer. Scheduled unread is a **filtered projection over the same regular watermark** (`scheduled_session_scope_sql` = legacy `is_cron=1` ∪ regular-scope rows with `origin.kind='cron'`), so `mark_all_cron_sessions_read` writes `sessions.last_read_message_id` and reading the chat clears both badges; it emits `cron:unread_changed`. Deleting a task is **logical** (`cron_jobs.deleted_at`): run logs and run chats are retained, `get_job` hides the row while `get_job_snapshot` still returns it for history surfaces. Run-history list paginates via `get_run_logs(limit, offset)` with `limit` clamped ≤200 at both shells.

### File ops / Project / Canvas — `file-operations.md`, `project.md`, `canvas.md`

- Entry: `filesystem/workspace.rs` (`WorkspaceScope`), `filesystem/ops.rs`, `ha-server/routes/sessions.rs`, `project/files.rs`, `tool_canvas/`.
- State: `sessions.db` (projects/sessions), `memory.db` (project memory), `canvas/canvas.db`. Config: `filesystem.allow_remote_writes` (default false), `canvas.*`.
- Grep: `category='tool'` source `canvas` / file-op traces.
- Gotcha: HTTP `/api/fs/*` write 403 in server mode → `filesystem.allow_remote_writes` is off. Preview-by-path 403 → path neither referenced-by-tool-msg nor inside the session working dir (remote arbitrary-read guard). `for_path` scope is read-only — writes always rejected. Canvas snapshot/eval timeout on non-html content types is expected.

### Browser / macOS control / macOS permissions — `browser.md`, `macos-control.md`, `macos-permissions.md`

- Entry: `ha-browser/src/tool/`, `browser/cdp_backend.rs`, `browser_state.rs`, `ha-mac/src/lib.rs`, `ha-mac/src/tool.rs`, `src-tauri/src/macos_control.rs`. Panel step-timeline: `tool_actions.rs` (per-session in-memory ring, `browser:action` / `mac_control:action` events, `tool_recent_actions` owner query). macOS TCC layer: `permissions.rs` (`PERMISSION_DEFS` catalog, `check_item` / `request_item` / `folder_status`).
- State: no SQLite — `browser-profiles/` + `browser/{managed-runner,user-attach,runtime}/`, `mac-control/snapshots/` + `mac-control/diagnostics/`. Config: `AppConfig.browser` (`launchCircuit`, `profiles[*]`). macOS permissions hold no persistent state — they probe live TCC each call.
- Grep: `category IN ('browser','mac_control','permissions','tool_actions')`. Wire-audit: `category='browser'` source `hooks.spawn_broker` / `hooks.schedule_turn_finalize` / `hooks.cleanup_session` = 该二进制没调 `ha_browser::wire()`（首次未装配打一次 OnceLock warn，之后静默 no-op）——headless / 精简二进制上意料之外见到就查装配序列漏了 wire_features。
- Gotcha: desktop-only — `mac_control` returns `supported=false` off macOS-Tauri and needs live TCC perms (Accessibility + Screen Recording); start with `action='status'`/`'permissions'`. Browser launch failures → `launch_circuit.rs` trip + 3-tier fallback. Missing `browser:frame`/`mac_control:frame` EventBus frames = capture failed. Stale `el_N`/`ref_id` only valid within their originating snapshot. Panel timeline (`tool_actions`) is memory-only — empty after restart is normal, not data loss; entries are purged on session delete/burn, typed text is redacted to `text(N chars)` by design. **macOS TCC (`permissions.rs`)**: `request_item` dispatches by **`def.id`** (NOT `request_mode` — that field is catalog metadata for the frontend only); real branches are the NativePrompt ids, the two `automation` ids (osascript probe), and a `_` fallback (`open_settings_pane` + re-check, covering OpenSettings + None). Folder ids carry a `_folder` suffix (`desktop_folder`/`documents_folder`/`downloads_folder` — bare `desktop` etc. do not exist). NativePrompt items already past `NotDetermined` open Settings instead of re-prompting (`requestable_status_or_open`). Off macOS the whole module is a `cfg` stub returning unsupported.

### Dashboard / Recap / Awareness — `dashboard.md`, `recap.md`, `behavior-awareness.md`

- Entry: `dashboard/`, `recap/`, `awareness/`, `agent/mod.rs`.
- State: `recap/recap.db` (rebuildable, keyed by `last_message_ts`), `sessions.db` (`learning_events`), `logs.db`, `cron.db`. Config: `awareness.enabled`/`mode` (hard-gate), `recap.analysisAgent`/`language`.
- Grep: `category IN ('awareness','recap')`.
- Gotcha: Dashboard stats must filter `is_cron=0 AND parent_session_id IS NULL`. Awareness short-circuits for incognito sessions (also excluded from stats); digest auto-clears after 3 consecutive side_query failures. `awareness.enabled=false` is a hard-gate that ignores per-session overrides.

### Learning loop — Coding Improvement / Domain Workflow / Domain Quality / Domain Eval — `coding-improvement.md`, `domain-workflow.md`, `domain-quality.md`, `domain-eval.md`

- Entry: 机器（提案生成 / 蒸馏 / 预览 / 落盘 / 提升、fixture campaign 跑批、四道闸 soak）在 `ha-improve/src/{coding_improvement,domain_workflow,domain_eval,domain_quality}.rs`；**台账**（`impl SessionDB` 纯 SQL 层 + wire 类型 + 行映射）恒留 kernel `ha-core/src/{coding_improvement,domain_eval,domain_quality}.rs` 同名模块。kernel → ha-improve 单槽回调 `improve_hooks::ensure_coding_workflow_retro_for_run`（工作流终态记 coding retro）；owner 入口只在壳层薄封装。
- State: `sessions.db` (`coding_workflow_retros` / `coding_improvement_proposals` / `coding_eval_runs` / `domain_evidence_items` / `domain_eval_runs`)。
- Grep: `category='improve'` source `hooks` = `ha_improve` 未装配（首次未命中 OnceLock warn 一次，之后 `ensure_coding_workflow_retro_for_run` 静默返 `Ok(None)`）——headless / 精简二进制上见到就查装配序列漏了 `wire_features`；`category='learning'` (kernel `learning_events.rs` 发布面，dashboard 只读聚合)。
- Gotcha: **不扩权、不执行** — `domain_workflow_templates` 只述交付契约，`preview_domain_workflow` 只出 draft/preview，不建 run 不执行、不碰连接器。Domain Learning 复用 Coding Improvement 同一 proposal queue（禁平行队列），preview → apply → 用户显式 promotion 才落生产。**Domain Quality / eval 确定性只读**：不调 LLM、不写状态、不碰连接器，与 `coding_eval_runs` 不混用；coding release gate 不代替 domain quality gate。incognito 下 preview/evidence/quality/eval 拒绝或返空只读，不落 durable。

### Ask-user / Prompt system / Media generation — `ask-user.md`, `prompt-system.md`, `media-generation.md`

- 问答超时排查：先查请求顶层 `timeout_mode`，显式 `inherit` / `never` / `after` 覆盖旧每题提示；用户关闭自动超时时 `after` 明确拒绝。事件中的 `timeoutAt` 才是有效截止时间。结果的 `answers` 只含用户回答，超时默认方案只进 `fallback`；旧 `timedOut` 结果也不能显示成已回答或通过确认门。当前仍是阻塞提问，没有异步回投。
- Entry: `tools/ask_user_question.rs`, `ask_user/questions.rs`, ha-channel 的 `channel/worker/ask_user.rs`, `system_prompt/build.rs`, kernel `ha-core/src/media_gen/` (crud/resolve 配置面); 执行机器与工具 `ha-media/src/` (`media_gen/` executor+adapters+catalog, `{image_generate,audio_generate}/` chat-tool front-ends).
- State: `sessions.db` (`ask_user_questions`). Config: `ask_user_question_timeout_enabled` (default false = wait forever) / `_secs`, `mediaGen` (`providers[]` + per-function `chains` — image / speech / music / sfx, each = primary model + fallbacks; an empty chain falls back to provider order — plus `imageDefaults` / `audioDefaults`).
- Catalog: new Google Nano Banana 2.1 preset permits 14 references and four panoramic ratios; Pro/saved configs remain unchanged. Google mask/video/PDF inputs remain unsupported. Synthetic HTTP bytes test serialization and parsing, not image quality or account acceptance.
- Design reference paths: image `design.create_artifact` appends paths within the shared 14-image total including existing inline inputs; the home composer still caps inline images at five. Check both execution paths when a catalog limit changes.
- Grep: `category='ask_user'`; media generation logs under `category='media_gen'` (source `resolve`/`execute`) plus tool-level `category='tool'` (source `image_generate`/`audio_generate`); IM ask_user uses `category='channel'` with an `ask_user:` prefix.
- Gotcha: pending ask-user is in-memory (`PENDING_ASK_USER_QUESTIONS` oneshot); startup flips all DB pending→answered, so a DB pending surviving a restart is an orphan (answering returns "No pending request"). Image / audio generation failures return a transparent per-candidate `failover_log` (one line per provider+model tried in the function chain, including capability-mismatch and endpoint-blocked skips). Provider credentials are owner-GUI-only — `update_settings` on `media_generation` accepts only `chains` / `imageDefaults` / `audioDefaults` and rejects any payload carrying `providers`.

### Reliability / Security / Sandbox / Platform / Backup — `reliability.md`, `security.md`, `sandbox.md`, `platform.md`, `backup-autosave.md`

- Entry: `guardian.rs`, `self_diagnosis.rs`, `toolchain_doctor.rs`, `crash_journal.rs`, `security/ssrf.rs`, `security/dangerous.rs`, `platform/mod.rs`, `sandbox.rs`, `docker/` (ha-vcs), `backup.rs` (`create_backup` / `snapshot_before_write` / `restore_*`).
- State: `crash_journal.json` (not SQLite), `sandbox.json`, `backups/backup_{ts}/` (full crash backups, keep `MAX_BACKUPS`=5) + `backups/autosave/` (per-write single-file snapshots, keep `MAX_AUTOSAVES`=50). Config: `guardian.enabled`, `permission.global_yolo` (+ CLI `--dangerously-skip-all-approvals`), `ssrf{}`, `searxng_docker_use_proxy`.
- Grep: SearXNG / Docker sandbox → `category='sandbox'` (source `docker` / `docker_archive` / `searxng`); permission decisions → `category='permission'`. Crash recovery / Guardian / `self_diagnosis` have **no** `logs.db` category — read `crash_journal.json` directly (the primary signal). Backup failures are warn-only under `category='config'`-adjacent paths (never block a legit write).
- Gotcha: crash/restart loop → read `crash_journal.json` (`crashes[].signal`, last `diagnosis_result`); Guardian does not supervise `server`/`acp`. Dangerous-skip must be checked via `security::dangerous::is_dangerous_skip_active()` (CLI flag OR `permission.global_yolo`), never the raw field. All outbound HTTP must pass `ssrf::check_url`. **Backup (`backup.rs`)**: two budget-isolated mechanisms — `auto_fix` restores a corrupt `config.json` from the **full crash backup** (`list_backups().first()`), NEVER from autosave; autosave is per-write single-file undo (filename encodes `{ts}__{kind}__{category}__{source}`, parsed by `splitn(4,"__")` — the `__` separator is a stable contract). `snapshot_before_write` swallows all errors (warn-only) so a broken safety-net can't block the user's write. The `ha-updater/src/backup.rs` (bare-binary store/prune) is a **completely separate** module — don't conflate.
- Docker 部署沙箱：`connectionError=socket_missing` 先检查父容器是否挂载可信的本地 socket；`permission_denied` 检查 socket GID 与容器附加组，禁止改成 `0666`。`HA_DEPLOYMENT=docker` 仅 `isolated` 可执行，成功归档上传可 grep `category='sandbox' AND source='docker_archive'`；其余 mode 的 `SandboxUnavailable` 是安全边界，不得回落宿主机。
- 自诊断请求：`self_diagnosis::call_llm` 的阻塞模型客户端使用 `apply_model_proxy_blocking`，禁止跟随3xx重发诊断提示词或`x-api-key`/Bearer；初始SSRF、代理、超时和失败用量记录保留。
- 自诊断终态：`ProviderBlocked`、`RequestContract`、`RetryDeferred` 等类型化终态立即结束诊断，不再尝试剩余服务商或离线 `basic_analysis`。Guardian 向标准错误流输出 `Diagnosis failed`，不自动修复、不写入新的 `diagnosis_result`；已有备份时间和崩溃记录保留，之后继续既有退避重启。只有无候选或候选全部以非终态错误失败才进入离线兜底，缺少诊断不能当作修复成功。
- Toolchain Doctor：设置 → 关于页与 `hope-agent doctor --json` 使用同一组固定探针，状态为 `detected/supported/degraded/blocked`。输出只保留版本、Docker daemon/context/socket 所有权类别、二进制来源类别和临时 wrapper 诊断码，清理 ANSI/control 并脱敏；它绝不安装、升级、启动服务、切 Docker context 或修改系统配置。
- **Backend freeze / "everything loading forever" (issue #433 Bug 2)** → `category='blocking'` source `run_blocking`. Every SQLite DB + config write is synchronous under a `Mutex`/write-lock; async Tauri commands / ha-server handlers MUST offload them via `ha_core::blocking::run_blocking` / `SessionDB::run` / `config::mutate_config_async` (`crates/ha-base/src/blocking.rs`, Layer C′ in `process-model.md`) — an inline sync DB/config call in an `async fn` pins a tokio worker for the whole lock-wait + file IO, and on a stalled filesystem (AV scan / cloud-synced home / slow disk) starves all `num_cpus` workers so the process stays alive but every command hangs. Any blocking op >5s logs a `blocking` warn naming the caller's closure (the culprit's module/function). Symptom: send-message spins + settings page stuck loading, no crash. If it reproduces, grep `category='blocking'` for the slow closure; also verify `~/.hope-agent` isn't on OneDrive/iCloud and no new `async fn` reintroduced an inline `.append_message(`/`mutate_config(`/DB call (the AGENTS.md red line).

### ACP / CLI / Slash commands / Config / Self-update — `acp.md`, `cli.md`, `slash-commands.md`, `config-system.md`, `self-update.md`

- Entry: `ha-acp/src/acp/agent.rs`, `slash_commands/handlers/` (assembly layer), `slash_defs/` (command table / wire types / parser / transcript rows), `slash_hooks.rs` (dispatch trampoline — IM goes through here, unregistered → `Err`), `config/persistence.rs`, `ha-updater/src/self_contained.rs`, `ha-updater/src/tool.rs`, `src-tauri/src/main.rs`.
- State: `sessions.db` (ACP shares SessionDB), `config.json`, `credentials/auth.json`. Config: `auto_update.*`, `permission.global_yolo`.
- Grep: `category IN ('self_update','acp','slash_cmd')`; config writes emit no log but fire the `config:changed` event.
- Gotcha: "UI save not taking effect until restart" class bugs ← a write that bypassed `mutate_config`. Binary swap must use `platform::atomic_replace_binary` + `--version` smoke test + auto-rollback. `MINISIGN_PUBKEY_BASE64` must equal `tauri.conf.json#plugins.updater.pubkey` or desktop startup panics. Headless auto-update loop only runs when `!is_desktop()`.
- Gotcha: isolated model evaluations keep Provider credentials only in the initialized process snapshot; `HA_MODEL_EVAL_MODE=1` rejects config writes and preserves that snapshot on reload. Repeated 401 errors after a background refresh require checking this boundary, without writing the test key to disk.
- Gotcha: Codex subscription requests reject `temperature`, including zero. A 400 before any tool execution is a request compatibility failure; the Codex adapter must omit the field even when the conversation/evaluation config sets it. Keep this protocol difference in baseline reports.
- Codex 的可选工具参数反复被填为空值时，检查请求和 Token 计量的 function schema 是否均显式携带 `strict: false`；不得在工具处理器里将任意无效参数吞掉。
- Workflow 因动态预览停在审批门时，检查脚本是否使用 `workflow.tool({ name, args })`；位置参数形式不属于 Host API 契约。按需 guide 必须提供完整格式，严格审批不能由测试脚本代答。
- 编排 suite `1.15.0` 的 `eval-control-plane-ready-v6` 由 owner 开启所需 Workflow Mode，为多 Agent 提供合成 Git 仓库，并把无人应答的问答限制为 30 秒后未回答到期。旧版模型停在 `enter_plan_mode`、缺少 Workflow 工具或无法创建子 worktree 时，先归因环境，再看模型任务；不得补造审批答案。control 的业务产物、机制与终态分开统计，不能把未跑故障臂当成恢复能力证据。
- 评测预算诊断中 `input_usage_unknown` / `output_usage_unknown` 表示调用用量未知而停止，只有实际已知累计用量达到上限才是 `input_tokens` / `output_tokens`。Codex 订阅的目录美元成本为零仍会消耗订阅额度。
- 终态墙钟必须包含初始化、判分与清理。`invalid_trial_evidence_after_execution` 表示已执行候选未通过契约校验，检查候选摘要和受控错误类别，保留用量且禁止自动重跑。当前 suite 的 Workflow 验证只读现有业务产物，需要审批的执行继续保持未批准。
- 清理时出现 `Query returned no rows` 先检查是否在途工作尚未停止就删除了合成 Session；隔离 cleanup 必须走正常 Stop、等待 trace 闭合后才删除，收敛失败则保留会话并报告清理失败。当前 suite 的 clean 初始 Goal 使用公开业务目标，不能把故障场景标题当成正常目标。
- 前台结束后仍有任务 span 时，核对实际父 Agent 结果通知回合。当前 suite 在原始单项预算内保持 root 归因，等待实际任务自然完成，连续静止 250 毫秒后才验收；等待及后续调用均计量。不能把通知回合当作 cosmetic 工作，或靠清理提升为通过；固定 250 毫秒等待不足以覆盖真实父回合。
- suite `1.15.1` 的 unlimited 诊断不使用注册时间截断回复后的父通知回合；排查提前收尾时同时检查请求超时、试次子进程与自然收尾观察三处，不能仅把预算对象清空。
- suite `1.15.0` / v6 的 `agent.result_queued` 在通知合并、idle 和重排时保留评测归因，排队不占运行并发；`agent.result_delivery` 只表示父通知回合，实际后台工具执行才计入 `asyncJobs` 和 `async_jobs` 信号。调查遗漏调用时同时检查这两种 span，不能只检查运行中的子 Agent。
- Gotcha: paired logical trial IDs are shared across models, while product runtime IDs include campaign and attempt. Registration conflicts or progress from an already finished model require checking this namespace; never use another campaign's telemetry as evidence for the current model.
- Gotcha: per-turn model overrides alone do not bind manual compaction or implicit children. Registered isolated evaluation sessions pin the selected model through the kernel Session service before any work starts. Check every `model.call` Provider/model binding, including summaries and earlier failed attempts; mixed-model evidence cannot validate the selected model.
- Gotcha: pre-compaction Memory Extract follows Agent/global extraction overrides and otherwise captures the current conversation Provider/model; it must never choose the first configured Provider/model. A mixed `agent.side_query` immediately before `context.dedicated_summarize` requires checking this flush path separately from the summarizer.
- Gotcha: a session-not-found error with digits such as `403` in its UUID is not an HTTP auth rejection. Check numeric error classification before blaming the Provider when unrelated trials suddenly report no auth profile; only explicit HTTP/status context or a leading standard HTTP reason phrase may activate status-code cooldowns.
- Gotcha: cleanup failures must preserve observed usage and open diagnostic traces as `infra_error/cleanup_incomplete_after_evidence`; never replay an already billed task. A crashed worker can recover only its own campaign/attempt telemetry. Zero observed calls with incomplete attribution is unknown usage, not free execution, and must block unsafe budget-based retries. Control owner-state expectations use `expectedByFaultProfile.clean`; actual faulted events use `chaos`.

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


### 2026-10-12 安全依赖与导出诊断

- 模型/媒体/嵌入收到 3xx 时先核对显式配置的端点，禁止通过重新发送原密钥/输入来探测目标；模型客户端禁止重定向，资源 GET 的逐跳 SSRF 是另一条路径。参见 Provider、媒体与安全文档。
- PDF 解析失败沿文本回退处理；深层输入的受控错误与进程 abort 分别记录。quick-xml 自有/Excel 为 0.41，xcb 旧构建链与 GTK/glib 平台债不冒称已消除。
- 客户端 MP4 导出分别定位编码、异步封装与清理错误；成功须有最终 MP4 bytes，不能以抓帧数或 encoder flush 完成认定已交付。Mediabunny 仅为隔离候选，合成读回不证明平台 H.264 编码/播放可用，生产默认未切换。
- 裸二进制/桌面验签共同锁定 minisign-verify 0.3.0（2026-11-05）；验签失败先核 payload、签名、公钥及 legacy 模式，不放宽校验。原0.2.5也检查规范S，版本差异不能当漏洞证据；公开测试公钥夹具不替代真实安装验收。
- 桌面 updater 的 Rust/JS 分别固定 2.14.1 / 2.14.0（2026-11-06 发版核对，主次版本一致）；降级开关来自应用配置且固定 false，不通过 renderer 参数覆盖版本门，不关闭验签/重启确认。
- R2 镜像或 Linux 仓库发布在安装 rclone 时失败：检查共用 `scripts/install-rclone.sh` 的固定版本、大小和官方 SHA，以及离线 version/flags 步骤；大小/摘要错误在解压执行前拒绝，版本/flag错误在原子安装前拒绝。不改成 apt 或跳过核验，不为排障带入桶凭据或重发生产工作流。
- 桌面发布在编译前报 `Found version mismatched Tauri packages` → 对照 Cargo.lock 与 pnpm-lock.yaml 的全部官方插件，而非只看 tauri/API/updater；dialog、notification、process 同样要求 major.minor 配对。运行 `node scripts/verify-tauri-version-sync.mjs`，再按发布流程验证，不能用普通 Rust 测试通过代替 Tauri CLI 预检。
- Linux 桌面构建在 `tauri-plugin-deep-link 2.6.2` 的 `ParseOption` 报 E0063 缺字段：核对 `rust-ini 0.21.3` 及根 Cargo patch 的完整官方提交 `463977675da7bbb257dee59669b5e122c8d681db`；不要重跑不变源码或降级到已撤回 INI 版本。普通 Rust CI 不编译 Tauri 壳，应按发布流程先验证待发提交的四平台 dry-run，再创建新 tag。

### Kimi Basic 搜索迁移

排查 `web_search.kimi.basic` 时先核对既有 Key、是否设置国家/语言/时效过滤及后续已启用服务商；Basic 不支持这些过滤，明确错误不代表账号异常。固定国内 REST、SSRF 与禁止重定向失败不回退旧聊天模型。用量中模型/Token 缺失属于按次合同；`billable_call=null` 表示取消或响应未知，不能据此断言免费或自动重试。仅收集脱敏状态码与时间，不采集查询、Key、响应正文或真实账号材料。

排查 `web_search.perplexity` 时区分 HTTP/重定向拒绝、限量 JSON 读取失败、协议错误和合法空结果。引用沿既有 `citations` 顺序，新增来源元数据消费尚待不可信信封前置交付；无 URL 的摘要不当作引用。旧 Sonar 端点仍沿原配置，不能把原生 Agent `output` 示例当作该端点实测回执。解析失败/取消也记一次失败调用，Token 缺失不是零；不要收集查询、Key、响应正文或用本地夹具推断真实账单。

### Goal 历史 Workflow 修复后仍阻塞

检查 Goal final audit 的 blockers、ruleGate.hardBlockers 与 criteriaStatus，并保留失败和后续 run 的台账。`runtime_error` 失败只由失败后新建的同 Goal/session/criterion/revision 的 completed run 加其真实 validation_passed 解除；取消、审批阻塞、跨版本证据和后续 validation_failed 不解除。主编排专项 `1.15.2` 对齐此规则，历史结果不得重判或混作同版本比较。

### 评测中出现无归因的技能复盘费用

检查隔离 sessions.db 的 model_usage_events：skills.auto_review 必须具备实际 session_id 与 metadata.eval，并匹配所属试验模型。合成 automation:skills_auto_review 键可能导致 session_id 被置空、漏出试验费用及使用全局默认模型；不得把只核验已进入 trace 的调用当成全量证明。主编排 1.15.3 / 上下文 1.0.2 接入真实 Session、排队前后台守卫与 automation 模型 pin；未注册接纳的请求在网络前拒绝。保存中止批次与已知额外费用，不覆盖旧结果。

### 前台模型正确但通知回合混模

按试验逐一核对所有 model.call，包括前台结束后的 agent.result_delivery。ParentInjection 必须提交父会话 preferred model，评测 Kernel 再核对其模型 pin 并拒绝回退。业务验收通过不等于绑定有效；混模试验保留真实调用和费用，排除能力横比，参考主编排 1.15.4 / 上下文 1.0.3。

原生结果的 model_binding 阻断检查覆盖前台、自动调用与父通知所有 model.call；失败为 infra_error/model_binding_mismatch。业务 passed 不能覆盖绑定失败；费用与 trace 原样保留且不得自动重跑。

### 无预算评测清理超时且有晚到复盘费用

核对 cleanup HTTP 500、未关闭 model_automation.run span 和随后写入的 skills.auto_review 用量。主编排 1.15.5 / 上下文 1.0.4 在无预算模式等待所有已接纳后台模型调用自然结束后才取最终费用快照；不得通过缩短守卫生命周期、删除未结束会话或把缺失用量置零伪造闭环。旧 trial 的 infra_error 和晚到费用单独保留。

### 上下文专项结束后出现无归属 side_query

同时核对 Native trial completedAt 与隔离 model_usage_events 的 agent.side_query 时间；有成功用量但 session_id / metadata.eval 为空，说明侧查询存活超过根身份。主编排 1.15.6 / 上下文 1.0.5 在单次调用入口持有身份至用量落账，并在锁内拒绝已结束身份；不可把额外费用并入旧分数或声称已全量入账。保留诊断批次和已知费用，修复后先核对上下文专项，再推进完整组。

### 单次请求已开始但取消后没有用量

核对 model.call_scope 与 Provider 调用计量。主编排 1.15.7 / 上下文 1.0.6 对实际进入适配器的每个尝试持有独立用量守卫；调用方取消产生 callerCancelled=true、dispatchOutcome=unknown 且 token 为缺失值，不能填零或直接归因模型能力失败。正常完成不重复写入，未发送的准备工作不虚构请求；不确定传输拒绝自动重发。保留已中止诊断与费用下界，勿与新版本分数混合。

### 确定性专项评测的装配与样本前置条件

正式轨道包含 7 个套件、71 个用例。Dreaming 自动规划返回空集合时先检查独立 adapter 是否经 `ha_server::wire_features()` 装配；不得降低预期分组。语义检索准确率为零时检查合成嵌入模型是否已注册、selection 是否指向该模型及向量 v2 签名是否一致。领域 trace 中 `issues` / `duplicates` / `gaps` / `checks` / `attendees` 必须为数组。晋升样本须携带有效 suite、基础 fixture 与追加式版本锁，目标存在时继续拒绝覆盖。`verify-evidence --allow-local` 仅在没有 release tag 时容许 dirty 的 local 诊断，默认发布检查仍拒绝。

### 单 Agent 对照全部失败或重启故障造成其他试次报错

先核对提示词和 owner-state 验收是否同时禁止子 Agent 又要求 completed/killed 子执行器；编排 1.15.8 使用独立单 Agent 提示词和空子执行器精确验收。子执行器专属的崩溃、取消故障在单 Agent 臂标记不适用，调用数必须为零，不计作模型能力失败。强制重启前还有子执行器或后台请求时，核对 process.restart 的 interruptedWorkAtRestart；已完成调用仍保留，试次总费用须未知。进程重启必须持有共享服务独占锁；其他普通试次持共享锁，不能让同一 supervisor 的四个分片互相终止。修复前已付费或不确定请求单独保留，不能修改旧分数或补零费用。

### 重启控制器返回 401 或故障未激活

先区分 supervisor 的本地 HTTP 401 与 Provider 账户错误。1.15.9 修复 Authorization 头名大小写和 TCP 分段读取，仍拒绝错误或重复凭据。未激活故障不能证明恢复失败，应保留业务检查并归为 benchmark_defect；旧版 task_failed 原始记录不覆写，只在分析中解释其评测失效原因。

### 额度或进程中断后分片没有完整结果

先保留已发布 shard、逐项检查点和已脱敏用量。1.15.10 在 shard.json.trials 目录先持久化派发标记，再逐项原子发布结果；有有效结果时可在同一计划、同一运行时配置下复用，无结果但有派发标记时拒绝自动重放。旧版分片只在结束时写盘，不能由调用进度、聊天摘要或 SQL token 反推未保存的验收成绩。

### 取消分支被业务文件验收误判或研究范围没有数据

编排 1.16.0 / 上下文 1.0.9 区分正常业务交付与取消/purge 的负向产物验收。检查 expectedByFaultProfile、expectedAbsent、真实 cancelled/killed 终态和故障激活，不能用缺失文件单独判成功。研究输入必须具有日期和引用标识，范围边界、条目数与 exactFields.citations 一同校验。一次错误反馈后的 completed 可以是恢复成功；无真实故障仍是 benchmark_defect，不能修改历史结果或宣称权限/安全完整覆盖。同一对照对 seed 相同不代表 unlimited 计算量相同。

### 脚本用户修订被 active_stream 拒绝

若研究范围修订的 HTTP 500 明确来自 `active_stream ... from parent_injection`，说明上一阶段的结果通知尚未释放会话流，不是 Provider 额度或模型质量证据。编排 1.17.0 / 上下文 1.0.10 在非取消的草稿修订前等待真实工作结算，再发送下一条脚本消息。取消与重启场景不等待所有目标自然结束。保留旧原始结果和已知用量，不自动重放；脱敏 HTTP 原因与清理后 callerCancelled 分开诊断。

若草稿修订后文件检查通过但 Goal 仍引用旧契约，先核对脚本是否真正经过 owner 控制面修订，而非仅发送模型无权执行的改约文字。编排 1.18.0 / 上下文 1.0.11 记录 `scripted_owner_goal_revision` 和严格修订号检查；不要放宽完成审计或忽略审批、取消及阻塞历史来消除失败。

编排 1.19.0 / 上下文 1.0.12 的取消、拒绝审批脚本只等待父会话接纳权，不清空子任务。检查 `scripted_admission_*`、真实活动子任务及终态：只有类型化 `409 chat_admission_busy` 可重新接纳；500 中的 active_stream 文字本身不是自动重放依据。旧 HA-GL-003 的第三轮 Loop 故障与只读设置矛盾，预览应拒绝；新版只验证状态观测和一次实际工具环境反馈，不能据此宣称多轮 Loop 验收。

编排 1.20.0 的子执行取消通过无参数 `stop_parent` 用户步骤调用既有 owner Stop，并先检查真实 running 子执行归属。仅留下后台进程但子 Agent 已返回 completed，不构成“取消正在执行的子 Agent”的前置条件。若 Goal 改约后旧 runtime_error 再度阻塞，核对失败与其原修订成功验证是否已经匹配；已解决的历史不能复活，旧成功也不能替代当前修订验收，审批/取消边界不放宽。

编排 1.22.0 的取消前置条件同时要求父会话真实后台作业与活动子执行。前台子执行和父后台作业独立核验，二者成立后才派发 owner Stop；缺少前置条件记录 benchmark_defect，Loader 同时校验能力与信号声明。保留 killed 子执行和无遗留工作的独立验收。

编排 1.23.0 区分显式 killed 和 owner 会话 Stop 的 interrupted/session_paused；仅其他 interrupted 或正常 completed 不能冒充取消。核对 scripted_owner_stop 捕获的具体目标、同会话归属、Stop 后逐目标终态及无遗留工作，不能只用 stopped=true 或任意历史 killed 行证明成功。

Goal 改约后 LSP 错误无法被后续 clean diagnostics 或 validation 清除时，核对 diagnostic 的 metadata.runId、source id 前缀及持久化 Workflow 的 Goal/session/criterion/revision。诊断本身未携带 goalRevision 不代表无法归属；必须经真实 run 解析，不能用外部修订字段或创建时间跨修订消除阻塞。历史同修订内修复保留，但不能满足当前新契约。

领域质量复核无法清除同修订旧错误时，检查 domain_quality_runs.goal_revision 及回写 summary/check 的 goalRevision。该可空列由旧库兼容迁移追加，历史 NULL 不回填猜测；旧运行在改约后完成、用户 source metadata 或完成 stats 携带新修订都不能覆盖创建时归属。

### Design 视觉场景验证未完成

`visual_scenarios_incomplete` 表示固定三视口 runner 未执行已保存的自定义场景，或场景清单在捕获期间变化；不是浏览器失效。检查场景清单的路由、状态、场景数量/身份和自定义视口。保留用户清单与旧基线，不自动清空场景或接受默认页基线。当前仅默认场景可自动比较；完整场景状态到达证据仍待实现。
