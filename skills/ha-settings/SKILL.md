---
name: ha-settings
description: "Manage Hope Agent application settings through conversation. Use when the user wants to view or change any app configuration: theme, language, enhanced focus indicators, proxy, temperature, notifications, tool timeout, context compaction, automatic session titles, web search, GitHub issue reporting, memory, embedding, multimodal embedding, dreaming (offline memory consolidation), recap, behavior awareness, smart-mode approvals, plan mode, ask-user-question timeout, tool-result disk spill threshold, embedded server, ACP control plane, MCP subsystem (kill switch / concurrency / backoff), per-skill env vars, or any other setting visible in the Settings UI. Trigger phrases: 'change settings', 'configure proxy', 'set theme to dark', 'turn on enhanced focus indicators', 'turn off notifications', 'adjust temperature', 'show my settings', 'bind the server to all interfaces', 'enable smart mode', 'tune dreaming', 'disable mcp', 'show my channels'. Trigger even when the user doesn't explicitly say 'settings' — any intent to adjust app behavior qualifies."
always: true
---

# Settings — Application Configuration Management

Use `get_settings` and `update_settings` to read and modify settings. **Never edit config files directly.** Coverage matches the desktop Settings UI one-to-one for everything that doesn't carry secrets. The GUI-only zones — Providers / API Keys, IM Channel accounts (`channels`), MCP server configs (`mcp_servers`), the active model selection (`active_model` / `fallback_models`), the embedding model selection for both memory and knowledge-base vector search (`memory_embedding` / `knowledge_embedding` — model choice carries a background reembed side effect, like `active_model`), the knowledge-base chunking parameters (`knowledge_chunk` — changing chunk size/overlap re-chunks and re-embeds every space, same heavy reindex side effect; tuned under Settings → Knowledge → Advanced), the Speech-to-Text subsystem (`stt_providers` / `active_stt_model` / `stt_fallback_models`), and the Hooks system (`hooks`) — are configured only in the Settings UI (memory vector search under Settings → Memory; knowledge vector search under Settings → Knowledge) so credentials stay out of conversation logs and the model can't grant itself command execution.

## Risk Levels & Dual-Confirmation

Every response from `get_settings` / `update_settings` includes a `riskLevel` field. **Follow this workflow strictly**:

| Risk | Required before calling `update_settings` |
|------|-------------------------------------------|
| `low` | One-line summary of what you'll change is enough |
| `medium` | Show current value → new value, then proceed if the user has asked for it |
| `high` | **MUST** explicitly ask the user to confirm (e.g. "Are you sure you want to change X from A to B? This affects …"). Wait for explicit yes before writing. |

`get_settings({ category: "all" })` returns a `riskLevels` map grouping every category.

If the response includes `sideEffect`, surface it to the user (e.g. "this requires an app restart").

## Workflow

1. **Understand intent** — what does the user want to view or change?
2. **Read current** — `get_settings(category)`. Note `riskLevel` and `sideEffect`.
3. **Confirm** — low: brief summary. medium: diff. **high: explicit yes/no prompt.**
4. **Apply** — `update_settings(category, values)` with partial JSON.
5. **Report** — show the updated values and any side-effect note (e.g. restart needed).

## Tool Usage

### get_settings

```json
{ "category": "theme" }        // Read one category
{ "category": "all" }          // Overview + riskLevels map
```

### update_settings

```json
{ "category": "theme", "values": { "theme": "dark" } }
```

`values` uses partial merge — only include fields you want to change.

## Full Category Reference

### LOW risk — cosmetic / preference, trivially reversible

| Category | Fields |
|----------|--------|
| `user` | `name`, `avatar`, `gender`, `birthday`, `role`, `timezone`, `language`, `aiExperience`, `responseStyle`, `customInfo`, `autoSendPending`, `autoExpandThinking`, `autoCollapseCompletedTurns`, `serverMode`, `remoteServerUrl`, `remoteApiKey`, `weatherEnabled`, `weatherCity`, `weatherLatitude`, `weatherLongitude` |
| `theme` | `theme` (`auto`/`light`/`dark`) |
| `language` | `language` (`auto`/`zh`/`en`/…) |
| `focus_indicator` | `enhancedFocusIndicators` (bool, default `false`). Enables the stronger 2px focus outline for all input methods. When disabled, pointer/touch focus stays visually quiet while keyboard navigation keeps the lightweight focus indicator. System `prefers-contrast: more` and forced-colors modes still take precedence automatically. |
| `ui_effects` | `uiEffectsEnabled` |
| `prevent_sleep` | `preventSleep` (bool, default `false`). When `true`, the primary process keeps the host awake by holding an OS sleep assertion (macOS `caffeinate -i` / Linux logind inhibitor / Windows `ES_SYSTEM_REQUIRED`) so long-running tasks, downloads, and background work aren't interrupted by idle sleep. The display may still turn off; takes effect immediately, no restart needed. |
| `notification` | `enabled`, `showChatContent` (include assistant reply previews in chat-completion notices), `notifyOnBackgroundJobComplete` (bool, default `true`; R4: fire a desktop notification when a background job finishes, gated by `enabled` + only when the window is in the background) |
| `startup_notification` | `enabled` (default `true`), `windowSecs` (lookback for "active" chats, default 259200 = 72h), `globalMax` (cap on the number of chats **actually notified** per boot — applied after silencing / cooldown filters so they can't starve fresh chats; default 30), `cooldownSecs` (per-chat silence after a notice, default 1800 = 30 min), `crashLoopThreshold` (suppress entirely when `HOPE_AGENT_CRASH_COUNT >= N`, default 3). Drives the short "back online" notice fanned out to recently-active IM chats after every fresh process boot (see `channel::worker::startup_watcher`). Each send task waits up to 30s for its IM account worker to flip to running (covers OAuth-y handshakes) before bailing — a timeout does **not** burn cooldown, so the next boot retries. Per-account silencing lives on `ChannelAccountConfig.notify_startup` and must be edited in the Channels GUI (this skill cannot reach it). |
| `canvas` | `enabled`, `autoShow`, `defaultContentType` (e.g. `code` / `html`), `maxProjects`, `maxVersionsPerProject`, `panelWidth` |
| `image` | `maxImages` |
| `pdf` | `maxPdfs`, `maxVisionPages` |
| `image_generate` | `provider`, `model`, `defaultSize` (e.g. `1024x1024`), `timeoutSeconds`, `providers` (per-provider entries — `id`, `enabled`, `apiKey`, `baseUrl`). **Read responses redact `providers[*].apiKey` to `"[REDACTED]"`**, so the model can list configured providers but never sees existing keys; writes still flow through (so the user can ask the skill to set a key, but the skill won't echo it back on next read). For best UX prefer Settings → Image Generate. |
| `temperature` | `temperature` (0.0–2.0, null = API default) |
| `tool_timeout` | `toolTimeout` (seconds, 0 = unlimited) |
| `default_agent` | `defaultAgentId` (string id; `null` / empty falls back to hardcoded `"default"` agent) |
| `local_llm_auto_maintenance` | `enabled` (bool, default `true`). Background watchdog that re-preloads default Ollama chat / embedding models when they fall out of `ollama ps`, and pops a frontend dialog when their files vanish. Read also returns `userStoppedModels` (Ollama tags the user explicitly stopped via the UI) but that array is **read-only via this skill** — it's owned by the preload/stop UI flow. Disabling stops the watchdog entirely; it does not unload anything currently running. |

### MEDIUM risk — behavioral changes (cost, context, output quality)

| Category | Fields |
|----------|--------|
| `compact` | Master: `enabled`, `cacheTtlSecs` (default 300, max 900). Trim ratios: `softTrimRatio` (default 0.50), `hardClearRatio` (default 0.70). Reactive microcompact: `reactiveMicrocompactEnabled` (default true), `reactiveTriggerRatio` (default 0.75, range 0.50–0.95). Tool-result trimming: `toolPolicies` (HashMap mapping tool name → `eager`/`protect`), `maxToolResultContextShare` (default 0.3, range 0.1–0.6), `minPrunableToolChars` (default 20000), `softTrimMaxChars` / `softTrimHeadChars` / `softTrimTailChars` (default 6000/2000/2000), `hardClearEnabled`, `hardClearPlaceholder`. Recent boundary: `preserveRecentRounds` (default 4, range 1–12; protects recent message rounds, expands to the owning user turn only when that does not swallow prior execution rounds). Tier 3 summary: `summarizationModel` (provider:model override), `summarizationThreshold` (default 0.85), `summarizationTimeoutSecs` (default 60), `summaryMaxTokens` (default 4096), `maxHistoryShare` (default 0.5), `maxCompactionSummaryChars` (default 16000, range 4000–64000), `maxCompactionInjectedContextShare` (default 0.5, clamped to `maxHistoryShare`; combined budget for summary + ledger + recovery), `identifierPolicy` (`strict`/`off`/`custom`), `identifierInstructions`, `customInstructions`. Recovery: `recoveryEnabled`, `recoveryMaxFiles` (default 5), `recoveryMaxFileBytes` (default 16384). |
| `session_title` | `enabled`, `providerId`, `modelId` (null provider/model = use the chat model). When enabled, new sessions keep the first-message fallback title immediately, then run one LLM call after the first assistant reply to generate a concise title. Manual renames are never overwritten. |
| `memory_extract` | `autoExtract`, `extractProviderId`, `extractModelId`, `flushBeforeCompact`, `extractTokenThreshold` (default 8000), `extractTimeThresholdSecs` (default 300), `extractMessageThreshold` (default 10), `extractIdleTimeoutSecs` (default 1800), `enableReflection`, `extractClaims` (default true; next-gen Dreaming structured claim dual-write (beta) — also gates the Dashboard Claims view) |
| `memory_selection` | `enabled`, `threshold` (min candidates before LLM picks, default 8), `maxSelected` (default 5) |
| `memory_budget` | `totalChars` (int, default 10000), `coreMemoryFileChars` (int, default 8000 — cap per `memory.md` file), `sqliteEntryMaxChars` (int, default 500 — cap per rendered SQLite bullet), `sqliteSections.{userProfile,aboutUser,preferences,projectContext,references}` (defaults 1500/2000/2000/3000/1500; `userProfile` was renamed from `aboutYou` and the system-prompt heading from `## About You` to `## User Profile` — the old `aboutYou` key is still accepted for back-compat). Priority order: Guidelines > Agent `memory.md` > Global `memory.md` > SQLite. Reducing `totalChars` may hide parts of `memory.md` from the system prompt; full content is still retrievable via `recall_memory` / `memory_get`. |
| `embedding_cache` | `enabled`, `maxEntries` |
| `dedup` | `thresholdHigh` (default 0.02), `thresholdMerge` (default 0.012) |
| `hybrid_search` | `vectorWeight` (default 0.6), `textWeight` (default 0.4), `rrfK` (default 60.0) |
| `temporal_decay` | `enabled` (default false), `halfLifeDays` (default 30.0) |
| `mmr` | `enabled` (default true), `lambda` (default 0.7) |
| `multimodal` | `enabled` (default false), `modalities` (array of `image`/`audio`, defaults to both), `maxFileBytes` (default 10485760 / 10MB). Requires a multimodal-capable embedding provider — enabling without one produces empty vectors silently. |
| `dreaming` | Master: `enabled` (default true). Triggers: `idleTrigger.{enabled,idleMinutes}` (default true / 30 min), `cronTrigger.{enabled,cronExpr}` (default false / `0 3 * * *`), `manualEnabled` (Dashboard "Run now" button). Promotion: `promotion.{minScore,maxPromote}` (default 0.75 / 5). Window: `scopeDays` (default 1), `candidateLimit` (default 50). Narrative: `narrativeMaxTokens` (default 2048), `narrativeTimeoutSecs` (default 60), `modelOverride` (`ModelChain`; deprecated `narrativeModel` `provider:model` string still read if unset; null = `function_models.automation` → chat default). Profile: `profileSynthesis.{enabled (default true), maxLinesPerScope (default 12)}` (per-scope user-profile aggregation; manual runs an LLM rewrite). |
| `recap` | `modelOverride` (`ModelChain`; deprecated `analysisAgent` agent-id string still read if unset; null = `function_models.automation` → chat default), `language` (output language for AI-generated sections/titles; `null`/empty = follow interface language), `defaultRangeDays`, `facetConcurrency`, `maxSessionsPerReport`, `cacheRetentionDays` |
| `function_models` | `vision.{providerId, modelId}` (vision-bridge model, issue #434) — the model used to transcribe images to text when the **main** model can't see images (text-only, e.g. DeepSeek). `null` / omit = bridge disabled (images are dropped with an `[image omitted]` placeholder, the pre-existing behavior). Plain model reference, **no credentials** (the API key lives in the referenced provider). Opt-in: only engages when set. Each engaged image is one extra LLM call on the configured vision model (billed under the `vision` usage kind); incognito sessions run the bridge but skip the ledger. Pick a genuinely vision-capable model (`inputTypes` includes `image`) or the bridge stays off. |
| `awareness` | Master: `enabled` (default false), `mode` (`off`/`structured`/`llm_digest`, default `structured`). Window: `maxSessions` (default 6), `maxChars` (default 4000), `lookbackHours` (default 72), `activeWindowSecs` (default 120), `previewChars` (default 200). Filters: `sameAgentOnly`, `excludeCron`, `excludeChannel`, `excludeSubagents`. Refresh control: `dynamicEnabled` (default true), `minRefreshSecs` (default 20), `semanticHintRegex`, `refreshOnCompaction`. LLM digest mode (`mode: "llm_digest"`): `llmExtraction.{modelOverride (ModelChain; null = reuse the current chat agent's own side_query — cache-friendly; setting it switches to a dedicated model via crate::automation, trading away that cache reuse), minIntervalSecs (300), maxCandidates (5), digestMaxChars (1200), concurrency (2), perSessionInputChars (2000), inputLookbackHours (4), fallbackOnError, reuseSideQueryCache}`. |
| `knowledge_passive_recall` | `enabled` (default true), `topN` (default 5, 1–20), `maxChars` (default 800, 100–4000), `cacheTtlSecs` (default 120), `showSnippet` (default false). Read bridge ③: each user turn injects the top accessible-KB note **titles** as an untrusted reference block (retrieval-only, no LLM). On by default so attached knowledge spaces feel alive immediately; incognito sessions and KBs the session isn't attached to never surface. |
| `knowledge_search` | `textWeight` (default 0.4), `vectorWeight` (default 0.6), `rrfK` (default 60, 1–1000), `mmrLambda` (default 0.7, 0–1), `candidateMultiplier` (default 3, 1–10). Hybrid `note_search` ranking: keyword (BM25) + semantic (vector) over note chunks → RRF fusion → MMR diversity re-rank. Pure query-time, **no reindex**. Weights' ratio sets keyword↔semantic balance; `rrfK` smooths fusion; `mmrLambda` trades relevance vs. variety; `candidateMultiplier` sizes the pre-MMR pool. Defaults suit most libraries; send the defaults above to restore them. |
| `web_fetch` | `enabled`, `maxBytes` |
| `web_search` | `providers` (per-provider entries — `id` ∈ DuckDuckGo / Searxng / Brave / Perplexity / Google / Grok / Kimi / Tavily, `enabled`, `apiKey`, `apiKey2` (Google CX), `baseUrl` (Searxng instance)), `searxngDockerManaged`, `searxngDockerUseProxy`, `defaultResultCount` (default 5), `timeoutSeconds` (30), `cacheTtlMinutes` (15), `defaultCountry`, `defaultLanguage`, `defaultFreshness`. **Read responses redact `providers[*].apiKey` and `providers[*].apiKey2` to `"[REDACTED]"`**, so the model can describe what's configured without seeing existing keys. Writes still flow through so the skill can help the user provision a new key, but the value won't be echoed on subsequent reads. |
| `issue_reporting` | `enabled`, `owner`, `repo`, `apiBaseUrl`, `labelsByKind.{bug,feature,improvement}`, `maxEvidenceChars`, `duplicateCheckEnabled`. GitHub token is optional and stored separately in `~/.hope-agent/credentials/github-issue.json`; do not ask `update_settings` to write it. If no token is configured, `issue_report` falls back to the user's authenticated `gh` CLI. Use Settings UI token controls or the dedicated Tauri/HTTP commands for token save/clear/test. |
| `deferred_tools` | `enabled` |
| `async_tools` | `enabled`, `autoBackgroundSecs`, `maxJobSecs`, `maxConcurrentJobs` (default: hardware-derived `clamp(logical_cores − 2, 4, 16)`, `0` = unlimited; global cap on concurrent `run_in_background` jobs — each holds an OS thread; at the cap a new background request QUEUES, R7.1), `maxConcurrentJobsPerSession` (usize, default: hardware-derived ≈ 3/4 of the global cap, band [3,12], always below it; `0` = no per-session limit; R7.1 fairness tier — per-session share of the pool: extra jobs from the same session queue even when the global pool has room, so one session/IM chat can't monopolize every slot; auto-backgrounded jobs are counted but not refused), `retryEnabled` (bool, **default `false` — opt-in**; R7.4 — auto-retry a backgrounded job that fails, with exponential backoff. **Only idempotent re-runnable tools** (`web_search` / `web_fetch`) are ever retried — `exec` / `image_generate` and any side-effecting tool are NEVER auto-retried regardless of this switch (eligibility is a code-level allowlist); user cancels / policy denials / timeouts are never retried. Off by default because an eligible tool re-RUNS and `web_search` is often a *paid* provider, so retrying a deterministic failure would re-bill — the user opts into that), `maxRetryAttempts` (u32, default `3`, hard-capped at 10; total attempts incl. the first run, `1` = no retry), `completionMergeWindowSecs` (u64, default `3`, `0` = disabled; R4: when several background jobs in the same session finish within this window, their completion notifications merge into ONE injected turn instead of N billed turns — `Group` is the pre-merged special case), `maxQueuedJobs` (usize, default `256`, R9; bounded wait-queue length once all concurrency slots are full — beyond it a new background request hard-rejects. NOT an "unlimited" knob: each queued job pins a live context in RAM, so it's clamped at read to `[1, 4096]`, `0` is floored not unbounded), `outputTailBytes` (usize, default `8192`, R9; bytes of a running background `exec` job's latest stdout/stderr kept for live `job_status` inspection — clamped at read to `[256, 1048576]`), `wakeupMaxDelaySecs` (u64, default `86400`, R9; upper bound on a `schedule_wakeup` self-scheduled delay — the 10s lower floor is fixed/non-configurable; clamped at read to `[10, 604800]` (10s–7d)), `wakeupMaxPendingPerSession` (usize, default `5`, R9; per-session cap on pending `schedule_wakeup` wakeups — a structural reject, not a queue; clamped at read to `[1, 100]`), `inlineResultBytes`, `retentionSecs`, `orphanGraceSecs`, `jobStatusMaxWaitSecs` |
| `timeout_policy` | `modelRuntimeOverrides` (`allow` / `warn` / `ignore_when_user_unlimited`, default `warn`). Governs model-supplied runtime timeout arguments that can shorten or kill long-running work (`exec.timeout`, async `job_timeout_secs`, sub-agent / ACP `timeout_secs`, cron per-job `job_timeout_secs`). It does **not** affect short polling waits or network/connect timeouts. `allow` honors silently; `warn` honors but logs/emits metadata; `ignore_when_user_unlimited` ignores a positive model timeout only when the corresponding user/system runtime budget is `0` (unlimited). Positive user budgets still cap execution and model values may only tighten them. |
| `cron` | `maxConcurrent` (u32, default `5`, `0` = unlimited) — global cap on how many scheduled tasks (cron jobs) may execute at once. Each cron run is a full agent turn (it can spawn sub-agents / tools), so a herd of jobs all due at the same instant could otherwise spawn dozens of simultaneous LLM turns and trip provider rate limits. The scheduler acquires a slot **before** claiming a due job (slot-before-claim), so a job beyond the cap keeps its schedule and runs on the next 15s tick instead of skipping the occurrence. Manual `run now` bypasses the cap but its running marker still counts toward occupancy. `jobTimeoutSecs` (u64, default `0` = unlimited; positive values are clamped at read to `[30, 7200]`) — global per-run wall-clock budget; on expiry the run is abandoned (logged as a `timeout` failure) and its slot freed. A per-job override (`CronJob.job_timeout_secs`, set via the cron job form / `manage_cron`, **not** this settings category) takes precedence for a single long-running task; under `timeout_policy.modelRuntimeOverrides = "ignore_when_user_unlimited"`, a positive model-supplied per-job override is ignored when the effective user/system cron timeout is unlimited. `atGraceSecs` (u64, default `300` = 5min; capped at read to 7 days; `0` preserved = strict, no late-fire) — late-fire grace window for one-shot `at` jobs: on startup an `at` job that came due while the app was down still fires if it's past-due by no more than this many seconds; beyond that it's marked `missed`. Unlike the other knobs `0` is NOT floored (it means strict miss). |
| `approval` | `approvalTimeoutEnabled` (bool, default `false`; when `false`, approval waits forever and `approvalTimeoutSecs` is only a saved duration), `approvalTimeoutSecs` (seconds, default 300; used only when `approvalTimeoutEnabled=true`), `approvalTimeoutAction` (`deny`/`proceed`) |
| `tool_result_disk_threshold` | `toolResultDiskThreshold` (bytes, null = default 50KB, 0 = disable) |
| `ask_user_question_timeout` | `askUserQuestionTimeoutEnabled` (bool, default `false`; when `false`, ask-user questions wait forever and model-provided `timeout_secs` is ignored), `askUserQuestionTimeoutSecs` (seconds, default 1800; used only when `askUserQuestionTimeoutEnabled=true`; `0` also waits forever) |
| `plan` | `planSubagent` (bool), `plansDirectory` (string or null) |
| `skills_auto_review` | Five-gate auto-review pipeline. Trigger / quality-floor fields (`enabled`, `promotion` (`draft`/`auto` — HIGH-equivalent), `cooldownSecs`, `tokenThreshold`, `messageThreshold`, `toolUseThreshold`, `correctionSignalEnabled`, `requireToolUse`, `minMessageCount`, `discardBlacklistDays`, `topKForDedup`, `minReuseProbability`, `sessionRecapThreshold`, `minSteps`/`maxSteps`, `candidateLimit`, `timeoutSecs`, `retentionDays`, `autoCuratorEnabled`, `autoCuratorIntervalDays`) are safe to tune. ⚠️ `reviewSystemOverride` replaces the built-in review prompt verbatim, and `extraRejectCategories` appends free-form reject categories — backend gates 2/4/5 still apply but the prompt-level safety net narrows. `modelOverride` (`ModelChain`; deprecated `reviewModel` `"provider:model"` string still read if unset) pins a dedicated review LLM. Confirm with the user before touching the three advanced fields. |
| `recall_summary` | `enabled`, `minHits`, `contextCharBudget`, `timeoutSecs`, `maxTokens`, `includeHistory`, `modelOverride` (`ModelChain`; null = `function_models.automation` → chat default) (Phase B'3 — opt-in LLM summarization on `recall_memory` output; adds one call per qualifying search, degrades silently on failure) |
| `tool_call_narration` | `toolCallNarrationEnabled` (bool, default `false`). When `true`, the system prompt tells the model to preface every tool call with a one-sentence announcement (Claude Code style). Some models (e.g. GPT-5.4 via Codex) over-apply this and restate identical intent across consecutive tool calls, causing visible duplication — default is off so users opt in explicitly. |
| `teams` | **Special: DB rows, not AppConfig fields.** `read` returns an array of all user-configured team templates. `update` uses CRUD-style values — `{ "action": "save", "template": {...} }` or `{ "action": "delete", "templateId": "..." }`. Saved templates become discoverable by the model via `team(action="list_templates")`. See "Special: `teams` semantics" below. |
| `im_auto_transcribe` | **Aggregate view + writer** for IM-channel voice auto-transcribe. Read returns `{ imFallbackModel, accounts: [{ id, label, channelId, autoTranscribeVoice }] }`. Write accepts `{ imFallbackModel?: { providerId, modelId } \| null, accounts?: [{ id, autoTranscribeVoice }] }` — both keys are independently optional. Enabling auto-transcribe consumes STT API quota per inbound voice message; without `imFallbackModel` (or `stt.activeModel` as fallback), the dispatcher logs a warning and forwards the original audio unchanged. Transcripts are prepended to the engine message as `[Voice transcript] …\n\n` (localised to `cfg.language`); the original audio always stays as an attachment alongside. |
| `knowledge_vision` | `modelOverride` (`ModelChain`; null = `function_models.automation` → chat default, filtered to vision-capable candidates only — non-vision models in the chain are silently skipped, not treated as failures), `timeoutSecs` (default 90, budgets the whole degradation attempt not one candidate), `maxTokens` (default 4096), `ocrConcurrency` (u8, default 3, clamped `[1,8]`; bounded concurrency for the scanned-PDF OCR fallback's per-page vision calls), `maxOcrPages` (usize, default 40, clamped `[1,120]`; page cap for the scanned-PDF OCR fallback — pages beyond it are silently truncated, recorded in the source's `Original-Total-Pages` header). Model selection for Knowledge Space image OCR import (both the Sources panel batch import and chat "Archive to Knowledge") plus the scanned-PDF (no text layer) OCR fallback. |
| `note_tools` | `modelOverride` (`ModelChain`; null = `function_models.automation` → chat default). Shared model chain for the three standalone note-authoring tools (`note_distill` / `note_moc` / `session_to_note`) — one field covers all three since they share one code path. |

### HIGH risk — require **explicit user confirmation**

| Category | Fields | Why high risk |
|----------|--------|---------------|
| `proxy` | `mode`, `url` | Affects ALL outgoing HTTP |
| `shortcuts` | `bindings` (array) | Global OS keybindings, can collide |
| `skills` | `extraSkillsDirs`, `disabledSkills`, `skillEnvCheck`, `allowRemoteInstall` | Disabling skills removes tools; `allowRemoteInstall` opens the HTTP `/api/skills/{name}/install` route that spawns `brew`/`npm -g`/`go install`/`uv tool install` — effectively RCE over the API Key |
| `server` | `bindAddr` (e.g. `127.0.0.1:8420` vs `0.0.0.0:8420`), `apiKey`, `publicBaseUrl`. **Read responses redact `apiKey` to `"[REDACTED]"`** so the bearer token isn't echoed back on every overview; writes still flow through. | Network exposure, requires app restart |
| `acp_control` | `enabled`, `backends` (each: `id`, `name`, `binary`, `acpArgs`, `enabled`, `defaultModel`, `env`), `maxConcurrentSessions`, `defaultTimeoutSecs`, `runtimeTtlSecs`, `autoDiscover`. **Read responses redact non-empty `backends[*].env` to `"[REDACTED]"`** because env frequently carries `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` overrides. | Controls external agent delegation |
| `skill_env` | Per-skill env vars (may contain secrets) | Stored plaintext in `config.json` |
| `security.ssrf` | `defaultPolicy` (`strict`/`default`/`allowPrivate`), `trustedHosts` (array), per-tool overrides `browserPolicy` / `webFetchPolicy` / `imageGeneratePolicy` / `urlPreviewPolicy` | Controls whether tools can reach private networks / cloud metadata. Relaxing policy or adding untrusted hosts enables SSRF attack paths |
| `security` | `skipAllApprovals` (bool) | ⚠️ **DANGEROUS MODE** — globally bypasses every tool approval gate (exec / write / edit / apply_patch / channel tools / browser / canvas). Overrides all per-session and per-channel auto-approve settings. Plan Mode restrictions still apply. A CLI flag `--dangerously-skip-all-approvals` can set this ephemerally without touching config; this field is the *persisted* switch. Treat with extreme caution and confirm twice |
| `unattended_approval` | `unattendedApprovalAction` (`deny`/`proceed`, default `deny`) | What to do when a tool needs approval on a surface where **no human can answer** — a cron run, a headless server with no connected client and no IM-attached chat, an ACP client without a permission capability, or a subagent with no parent surface. `deny` (default) fail-closes with a structured reason (the safe default — nothing hangs). ⚠️ `proceed` **auto-runs** those tools with no human in the loop — narrower than full YOLO (only fires on genuinely unattended surfaces, not every interactive approval), but still a security loosening. Confirm before switching to `proceed`. |
| `smart_mode` | `strategy` (`self_confidence` / `judge_model` / `both`), `judgeModel.{providerId, model, extraPrompt}` (required when strategy ∈ {judge_model, both}), `fallback` (`default` / `ask` / `allow`) | Reshapes which tool calls auto-approve in any session running `permission_mode = smart`. `judge_model` / `both` issue an extra side_query (5s hard timeout, 60s TTL) per approvable call — picking a slow / expensive model affects cost and latency across the board. `fallback: "allow"` can silently approve tools when the judge is unreachable. |
| `mcp_global` | `enabled`, `maxConcurrentCalls`, `backoffInitialSecs`, `backoffMaxSecs`, `consecutiveFailureCircuitBreaker`, `autoReconnectAfterCircuitSecs`, `deniedServers` (array of server-name strings) | MCP subsystem kill switch + concurrency caps + reconnect/backoff tuning + enterprise deny-list. Flipping `enabled=false` short-circuits every dispatch on next call (existing sessions stay open until they idle out); `deniedServers` additions prevent users from adding specific server names; loosening the backoff / circuit-breaker settings can cause aggressive retry storms against an upstream server. `alwaysLoad` is a per-server attribute on `mcp_servers`, not a `mcp_global` field. |
| `filesystem` | `allowRemoteWrites` (bool) | File-browser write gate for the HTTP/WS transport. Default `false`: remote token-bearing clients get read-only browsing, while the desktop (Tauri IPC) always writes. Enabling lets any HTTP client create / edit / delete / rename / upload files inside the project working directory on the **server host** — network-exposure risk, confirm before flipping on. |
| `browser` | `backendPreference` (`extension_first` / `cdp_only` / `extension_only`), `extension.{enabled, allowRawCdp, nativeHostName, extensionIds, showControlOverlay}`, `heartbeatIntervalSecs`, `defaultMode` (`managed` / `user_attach`). **Field-level merge** — pass only what you change; `profiles` / `launchCircuit` are complex structures better edited in the GUI Browser panel. | **HIGH** — `extension.enabled` + `backendPreference` decide whether browser actions drive the user's **real logged-in Chrome** (all cookies / sessions) via the extension, or an isolated CDP Chrome. `extension.allowRawCdp` is the kill switch for the raw DevTools Protocol escape hatch (arbitrary CDP against the real browser; each call is strict-approved). Treat as security-sensitive — confirm before enabling extension access or toggling raw CDP. Takes effect on subsequent browser actions. |
| `knowledge_maintenance` | `enabled`, `idleTrigger.{enabled,idleMinutes}`, `cronTrigger.{enabled,cronExpr}`, `manualEnabled`, `tasks.{autoLink,orphanRescue,frontmatterFill,dedupMerge,knowledgeGap,autoTag,mocUpkeep,memoryToNote,sourceCompile,sourceConflict,openQuestionsMoc,forAgentSummary}`, `autoApprove`, `maxProposalsPerCycle`, `dedupSimilarity`, `llmTimeoutSecs`, `llmMaxTokens`, `modelOverride` (`ModelChain`; shared by the 4 LLM-backed generators — autoTag/mocUpkeep/memoryToNote/sourceConflict; null = `function_models.automation` → chat default) | Layer-2 autonomous maintenance: background cycles scan knowledge bases and queue note-maintenance **proposals** for owner review (knowledge view → maintenance panel). ⚠️ `enabled` lets background cycles run; `autoApprove=true` writes approved-free changes **directly to the user's notes** (skipping review). Compile-class source suggestions still ignore auto-approve and only create compile Review Diff proposals after explicit approval. Both are approval-policy / autonomous-write switches — confirm with the user before enabling either. Takes effect on the next cycle. |
| `knowledge_media_retention` | `enabled` (default false), `maxTotalBytes` (default 1073741824), `maxSourceBytes` (default 209715200), `thumbnailMaxEdgePx` (default 512), `pruneWhenOverQuota` (default true) | Optional original-media retention for Knowledge Compiler sources. Enabling stores imported audio/video/image originals and image thumbnails under Hope's internal knowledge source directory. HIGH/privacy: confirm before enabling, raising quota, or changing prune behavior. |
| `sprite` | `enabled`, `proactive`, `triggers.{editIdle,noteOpen,conversation,periodic,paste}`, `idleEditSecs`, `minChangeChars`, `periodicSecs`, `pasteMinChars`, `cooldownSecs`, `maxPerSessionPerHour`, `senses.{doc,edit,conversation,memory,awareness}`, `maxTokens`, `timeoutSecs`, `modelOverride` (`ModelChain`; null = `function_models.automation` → chat default — fire-and-forget call, a full fallback chain is fine, not just a single model) | Knowledge-space sprite / inspiration mode: while the user works on a note, it makes a **bounded, proactive LLM call** on several occasions and may surface a transient suggestion bubble in the chat panel. ⚠️ `enabled` makes unprompted LLM calls — has a cost (users normally toggle it from the chat panel's sprite button, not here). `proactive` (default true) biases speaking vs. staying quiet; `triggers.*` toggle the occasions (edit-idle / note-open / after a chat turn / periodic while writing / large paste); `idleEditSecs`+`minChangeChars` gate edit-idle, `periodicSecs` the periodic streak, `pasteMinChars` the paste trigger; `cooldownSecs` / `maxPerSessionPerHour` throttle overall, `senses.*` toggle which context is fused. The persona prompt is built-in (English), not free-text configurable. Confirm with the user before enabling. Incognito sessions never trigger it. Only fires while the knowledge-space AI chat panel is open. |
| `auto_update` | `checkEnabled` (bool), `checkIntervalHours` (u32, clamped to `[1, 168]`), `autoDownload` (bool), `notify` (bool) | Background update behavior shared by desktop + headless. `checkEnabled` reaches out to the release server on a timer; `autoDownload` silently pre-fetches **and Minisign-verifies** the new binary into staging so the install is a no-network swap; `notify` surfaces "update available". The actual binary swap / restart **always** stays behind the user-confirmed `app_update install` (headless) or the GUI restart choice (desktop) — this category only governs the check + pre-download. Confirm before enabling/raising cadence (network reach-out + bandwidth). |

### Read-only (cannot be modified via this tool)

| Category | Description |
|----------|-------------|
| `active_model` | Current primary model — use Settings UI |
| `fallback_models` | Fallback chain — use Settings UI |
| `embedding` | Active memory-embedding config. Read resolves the currently-selected model from the shared `embedding_models` library + `memory_embedding` selection (the same source the GUI and runtime use) and returns `enabled` / `providerType` / `apiBaseUrl` / `apiModel` / `apiDimensions` with **`apiKey` redacted** (`"[REDACTED]"`); a disabled selection reads as `enabled:false`. Writes are GUI-only (Settings → Memory) — the model choice carries an API key and a heavy background reembed side effect, same class as `active_model` / `memory_embedding` / `knowledge_embedding`. |
| `channels` | IM Channel accounts (Telegram / WeChat / Feishu / QQ / Discord). Read returns the account list with **`credentials` and `settings` fields redacted** (`"[REDACTED]"`); structural metadata (`id`, `channelId`, `label`, `enabled`, `agentId`, `autoApproveTools`, `security`) is exposed so the model can reference accounts without seeing bot tokens. Writes must go through Settings → Channels so the registry can drop/re-establish listeners under user supervision and credentials stay out of conversation logs. |
| `mcp_servers` | MCP server configs. Read returns the server list with **`env`, `headers`, `oauth` fields redacted**. Writes must go through Settings → MCP Servers UI which enforces "trust acknowledgement" for stdio servers and routes credentials through `platform::write_secure_file` (0600). |
| `hooks` | Hooks system (Claude Code compatible). Read returns `{ disableAllHooks, hooks }` with **http handler `headers` values redacted**. **Read-only here on purpose** — hooks run arbitrary commands / HTTP / LLM prompts / sub-agents on lifecycle events, so a writable category would let the model persist its own command execution (privilege escalation). Edit in Settings → Hooks or the scope files (user: `config.json`; project: `<working_dir>/.hope-agent/hooks.json`, repo-shared; local: `hooks.local.json`, git-ignored; managed: `/etc/hope-agent/hooks.json`). All scopes are UNIONed. |
| `stt_providers` | Speech-to-Text providers (cloud + local servers). Read returns the provider list with **`apiKey`, `authProfiles[*].apiKey` redacted** and **the entire `extra` map replaced with `"[REDACTED]"`** (covers Volcengine `app_id` / `access_key`, iFlytek `app_id`, Azure region key, etc.). Writes must go through Settings → Speech-to-Text so credentials stay out of conversation logs. |
| `active_stt_model` | Active STT model for desktop voice input — use Settings UI so the engine cache picks up the new selection without an app restart. |
| `stt_fallback_models` | STT failover chain — use Settings UI. |

Model / Provider / API Key / IM Channel accounts / MCP server configs / STT providers / per-session configs require the Settings UI.

## Special: `teams` Semantics

Unlike every other category, `teams` does **not** live in `AppConfig` — it targets rows in the `team_templates` SQLite table. The `update_settings` payload is CRUD-shaped:

```json
// Create or overwrite a template
{
  "category": "teams",
  "values": {
    "action": "save",
    "template": {
      "templateId": "fullstack-py-react",
      "name": "Full-Stack (Py + React)",
      "description": "Frontend (React expert) + Backend (Python expert) + Tester",
      "members": [
        {
          "name": "Frontend",
          "role": "worker",
          "agentId": "react-expert",
          "color": "#3B82F6",
          "description": "You are the frontend specialist. Build React components with TS.",
          "modelOverride": null,
          "defaultTaskTemplate": "Implement the UI for the feature."
        }
      ]
    }
  }
}

// Delete a template by id
{
  "category": "teams",
  "values": { "action": "delete", "templateId": "fullstack-py-react" }
}
```

- `read` returns the full `TeamTemplate[]` — no `values` needed.
- `templateId` must be non-empty and unique. Each member's `agentId` must point to an existing Agent (check `list_agents` in the Agents panel).
- Deleting a template does **not** touch any teams that were created from it; `teams.template_id` is a historical reference only.
- EventBus broadcasts `template_saved` / `template_deleted` so the UI refreshes live.

## Special: `skill_env` Update Modes

Because per-skill env vars are a nested map, `update_settings("skill_env", …)` accepts three patch forms:

```json
// 1. Full replace
{ "skillEnv": { "my-skill": { "API_KEY": "xyz" } } }

// 2. Per-skill set (merge) — value null removes that var
{ "set": { "my-skill": { "API_KEY": "xyz", "OLD_VAR": null } } }

// 3. Remove an entire skill's env block
{ "remove": ["my-skill"] }
```

Prefer form 2 for targeted edits so you don't overwrite unrelated skills.

## Rollback — Every Change Is Reversible

Every write to `config.json` / `user.json` — from this tool, the UI, or any other path — automatically snapshots the pre-change file under `~/.hope-agent/backups/autosave/`. Last 50 snapshots retained.

### list_settings_backups

```json
{ "limit": 10 }              // latest 10 entries (default 20, max 200)
{ "kind": "config" }         // filter by "config" or "user"
```

Returns `{id, timestamp, kind, category, source}` newest first.

### restore_settings_backup

```json
{ "id": "2026-04-17T10-30-45-123__config__theme__skill" }
```

- **Always HIGH risk** — must confirm with the user before calling. Show them the entry's `timestamp`, `kind`, and `category`.
- Creates a fresh snapshot of the current state first, so the rollback itself is reversible — you can "undo the undo" by restoring the newly-created entry.
- Restoring a `config` entry reloads the in-memory cache immediately; `server` / `shortcuts` style side effects still apply and may need a restart.

### When to proactively offer rollback

- User says "undo that", "revert", "go back", "you broke X" after a recent change.
- User complains about a specific behavior right after you changed a related setting.
- User asks "what did you change?" — list the last few entries to remind them.

## Important Notes

- **Read before write** — always `get_settings` first so you can show a diff.
- **Confirm before write** — especially HIGH risk. Include the risk level in your confirmation prompt.
- **Field names are camelCase** (e.g. `softRatio`, `toolTimeout`, `approvalTimeoutEnabled`, `askUserQuestionTimeoutEnabled`, `askUserQuestionTimeoutSecs`).
- **Security restrictions** — cannot modify Providers or API Keys through this tool; guide the user to the Settings UI.
- **Surface side effects** — if the response has `sideEffect` (e.g. "requires restart"), tell the user.
- **Secrets in logs** — never echo `apiKey`, `remoteApiKey`, or `skill_env` values back in chat unless the user explicitly asks. Note that `get_settings` for `server` / `web_search` / `image_generate` / `acp_control` / `embedding` already redacts the credential fields to `"[REDACTED]"` — if you see that marker, the field is set but the value is hidden from the model intentionally.
- **Rollback is built-in** — if a change goes wrong, offer `restore_settings_backup` instead of trying to reconstruct the old values manually.
