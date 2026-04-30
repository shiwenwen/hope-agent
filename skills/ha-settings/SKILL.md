---
name: ha-settings
description: "Manage Hope Agent application settings through conversation. Use when the user wants to view or change any app configuration: theme, language, proxy, temperature, notifications, tool timeout, context compaction, automatic session titles, web search, memory, embedding, multimodal embedding, dreaming (offline memory consolidation), recap, behavior awareness, smart-mode approvals, plan mode, ask-user-question timeout, tool-result disk spill threshold, embedded server, ACP control plane, MCP subsystem (kill switch / concurrency / backoff), per-skill env vars, or any other setting visible in the Settings UI. Trigger phrases: 'change settings', 'configure proxy', 'set theme to dark', 'turn off notifications', 'adjust temperature', 'show my settings', 'bind the server to all interfaces', 'enable smart mode', 'tune dreaming', 'disable mcp', 'show my channels'. Trigger even when the user doesn't explicitly say 'settings' — any intent to adjust app behavior qualifies."
always: true
---

# Settings — Application Configuration Management

Use `get_settings` and `update_settings` to read and modify settings. **Never edit config files directly.** Coverage matches the desktop Settings UI one-to-one for everything that doesn't carry secrets. The four GUI-only zones — Providers / API Keys, IM Channel accounts (`channels`), MCP server configs (`mcp_servers`), and the active model selection (`active_model` / `fallback_models`) — are still readable here (with credentials redacted where applicable) but writes must happen in the Settings UI so credentials stay out of conversation logs.

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
| `user` | `name`, `avatar`, `gender`, `birthday`, `role`, `timezone`, `language`, `aiExperience`, `responseStyle`, `customInfo`, `autoSendPending`, `autoExpandThinking`, `serverMode`, `remoteServerUrl`, `remoteApiKey`, `weatherEnabled`, `weatherCity`, `weatherLatitude`, `weatherLongitude` |
| `theme` | `theme` (`auto`/`light`/`dark`) |
| `language` | `language` (`auto`/`zh`/`en`/…) |
| `ui_effects` | `uiEffectsEnabled` |
| `notification` | `enabled` |
| `canvas` | `enabled`, `autoShow`, `defaultContentType` (e.g. `code` / `html`), `maxProjects`, `maxVersionsPerProject`, `panelWidth` |
| `image` | `maxImages` |
| `pdf` | `maxPdfs`, `maxVisionPages` |
| `image_generate` | `provider`, `model`, `defaultSize` (e.g. `1024x1024`), `timeoutSeconds`, `providers` (per-provider entries — `id`, `enabled`, `apiKey`, `baseUrl`). **Read responses redact `providers[*].apiKey` to `"[REDACTED]"`**, so the model can list configured providers but never sees existing keys; writes still flow through (so the user can ask the skill to set a key, but the skill won't echo it back on next read). For best UX prefer Settings → Image Generate. |
| `temperature` | `temperature` (0.0–2.0, null = API default) |
| `tool_timeout` | `toolTimeout` (seconds, 0 = unlimited) |
| `default_agent` | `defaultAgentId` (string id; `null` / empty falls back to hardcoded `"default"` agent) |

### MEDIUM risk — behavioral changes (cost, context, output quality)

| Category | Fields |
|----------|--------|
| `compact` | Master: `enabled`, `cacheTtlSecs` (default 300, max 900). Trim ratios: `softTrimRatio` (default 0.50), `hardClearRatio` (default 0.70). Reactive microcompact: `reactiveMicrocompactEnabled` (default true), `reactiveTriggerRatio` (default 0.75, range 0.50–0.95). Tool-result trimming: `toolPolicies` (HashMap mapping tool name → `eager`/`protect`), `maxToolResultContextShare` (default 0.3, range 0.1–0.6), `keepLastAssistants` (default 4), `minPrunableToolChars` (default 20000), `softTrimMaxChars` / `softTrimHeadChars` / `softTrimTailChars` (default 6000/2000/2000), `hardClearEnabled`, `hardClearPlaceholder`. Tier 3 summary: `summarizationModel` (provider:model override), `summarizationThreshold` (default 0.85), `preserveRecentTurns` (default 4, max 12), `summarizationTimeoutSecs` (default 60), `summaryMaxTokens` (default 4096), `maxHistoryShare` (default 0.5), `maxCompactionSummaryChars` (default 16000, range 4000–64000), `identifierPolicy` (`strict`/`off`/`custom`), `identifierInstructions`, `customInstructions`. Recovery: `recoveryEnabled`, `recoveryMaxFiles` (default 5), `recoveryMaxFileBytes` (default 16384). |
| `session_title` | `enabled`, `providerId`, `modelId` (null provider/model = use the chat model). When enabled, new sessions keep the first-message fallback title immediately, then run one LLM call after the first assistant reply to generate a concise title. Manual renames are never overwritten. |
| `memory_extract` | `autoExtract`, `extractProviderId`, `extractModelId`, `flushBeforeCompact`, `extractTokenThreshold` (default 8000), `extractTimeThresholdSecs` (default 300), `extractMessageThreshold` (default 10), `extractIdleTimeoutSecs` (default 1800), `enableReflection` |
| `memory_selection` | `enabled`, `threshold` (min candidates before LLM picks, default 8), `maxSelected` (default 5) |
| `memory_budget` | `totalChars` (int, default 10000), `coreMemoryFileChars` (int, default 8000 — cap per `memory.md` file), `sqliteEntryMaxChars` (int, default 500 — cap per rendered SQLite bullet), `sqliteSections.{userProfile,aboutUser,preferences,projectContext,references}` (defaults 1500/2000/2000/3000/1500; `userProfile` was renamed from `aboutYou` and the system-prompt heading from `## About You` to `## User Profile` — the old `aboutYou` key is still accepted for back-compat). Priority order: Guidelines > Agent `memory.md` > Global `memory.md` > SQLite. Reducing `totalChars` may hide parts of `memory.md` from the system prompt; full content is still retrievable via `recall_memory` / `memory_get`. |
| `embedding_cache` | `enabled`, `maxEntries` |
| `dedup` | `thresholdHigh` (default 0.02), `thresholdMerge` (default 0.012) |
| `hybrid_search` | `vectorWeight` (default 0.6), `textWeight` (default 0.4), `rrfK` (default 60.0) |
| `temporal_decay` | `enabled` (default false), `halfLifeDays` (default 30.0) |
| `mmr` | `enabled` (default true), `lambda` (default 0.7) |
| `multimodal` | `enabled` (default false), `modalities` (array of `image`/`audio`, defaults to both), `maxFileBytes` (default 10485760 / 10MB). Requires a multimodal-capable embedding provider — enabling without one produces empty vectors silently. |
| `dreaming` | Master: `enabled` (default true). Triggers: `idleTrigger.{enabled,idleMinutes}` (default true / 30 min), `cronTrigger.{enabled,cronExpr}` (default false / `0 3 * * *`), `manualEnabled` (Dashboard "Run now" button). Promotion: `promotion.{minScore,maxPromote}` (default 0.75 / 5). Window: `scopeDays` (default 1), `candidateLimit` (default 50). Narrative: `narrativeMaxTokens` (default 2048), `narrativeTimeoutSecs` (default 60), `narrativeModel` (`provider:model` override; null = active chat agent). |
| `recap` | `analysisAgent`, `defaultRangeDays`, `facetConcurrency`, `maxSessionsPerReport`, `cacheRetentionDays` |
| `awareness` | Master: `enabled` (default false), `mode` (`off`/`structured`/`llm_digest`, default `structured`). Window: `maxSessions` (default 6), `maxChars` (default 4000), `lookbackHours` (default 72), `activeWindowSecs` (default 120), `previewChars` (default 200). Filters: `sameAgentOnly`, `excludeCron`, `excludeChannel`, `excludeSubagents`. Refresh control: `dynamicEnabled` (default true), `minRefreshSecs` (default 20), `semanticHintRegex`, `refreshOnCompaction`. LLM digest mode (`mode: "llm_digest"`): `llmExtraction.{extractionAgent, extractionModel, minIntervalSecs (300), maxCandidates (5), digestMaxChars (1200), concurrency (2), perSessionInputChars (2000), inputLookbackHours (4), fallbackOnError, reuseSideQueryCache}`. |
| `web_fetch` | `enabled`, `maxBytes` |
| `web_search` | `providers` (per-provider entries — `id` ∈ DuckDuckGo / Searxng / Brave / Perplexity / Google / Grok / Kimi / Tavily, `enabled`, `apiKey`, `apiKey2` (Google CX), `baseUrl` (Searxng instance)), `searxngDockerManaged`, `searxngDockerUseProxy`, `defaultResultCount` (default 5), `timeoutSeconds` (30), `cacheTtlMinutes` (15), `defaultCountry`, `defaultLanguage`, `defaultFreshness`. **Read responses redact `providers[*].apiKey` and `providers[*].apiKey2` to `"[REDACTED]"`**, so the model can describe what's configured without seeing existing keys. Writes still flow through so the skill can help the user provision a new key, but the value won't be echoed on subsequent reads. |
| `deferred_tools` | `enabled` |
| `async_tools` | `enabled`, `autoBackgroundSecs`, `maxJobSecs`, `inlineResultBytes`, `retentionSecs`, `orphanGraceSecs`, `jobStatusMaxWaitSecs` |
| `approval` | `approvalTimeoutSecs`, `approvalTimeoutAction` (`deny`/`proceed`) |
| `tool_result_disk_threshold` | `toolResultDiskThreshold` (bytes, null = default 50KB, 0 = disable) |
| `ask_user_question_timeout` | `askUserQuestionTimeoutSecs` (0 = wait forever) |
| `plan` | `planSubagent` (bool), `plansDirectory` (string or null) |
| `skills_auto_review` | `enabled`, `promotion` (`draft`/`auto`), `cooldownSecs`, `tokenThreshold`, `messageThreshold`, `timeoutSecs`, `candidateLimit` (Phase B'1 — when `promotion: "auto"` skip draft review; treat that as HIGH-equivalent and confirm with user) |
| `recall_summary` | `enabled`, `minHits`, `contextCharBudget`, `timeoutSecs`, `maxTokens`, `includeHistory` (Phase B'3 — opt-in LLM summarization on `recall_memory` output; adds one side_query per call, degrades silently on failure) |
| `tool_call_narration` | `toolCallNarrationEnabled` (bool, default `false`). When `true`, the system prompt tells the model to preface every tool call with a one-sentence announcement (Claude Code style). Some models (e.g. GPT-5.4 via Codex) over-apply this and restate identical intent across consecutive tool calls, causing visible duplication — default is off so users opt in explicitly. |
| `teams` | **Special: DB rows, not AppConfig fields.** `read` returns an array of all user-configured team templates. `update` uses CRUD-style values — `{ "action": "save", "template": {...} }` or `{ "action": "delete", "templateId": "..." }`. Saved templates become discoverable by the model via `team(action="list_templates")`. See "Special: `teams` semantics" below. |

### HIGH risk — require **explicit user confirmation**

| Category | Fields | Why high risk |
|----------|--------|---------------|
| `proxy` | `mode`, `url` | Affects ALL outgoing HTTP |
| `embedding` | `provider`, `model`, `dimensions` | May invalidate existing vector indexes |
| `shortcuts` | `bindings` (array) | Global OS keybindings, can collide |
| `skills` | `extraSkillsDirs`, `disabledSkills`, `skillEnvCheck`, `allowRemoteInstall` | Disabling skills removes tools; `allowRemoteInstall` opens the HTTP `/api/skills/{name}/install` route that spawns `brew`/`npm -g`/`go install`/`uv tool install` — effectively RCE over the API Key |
| `server` | `bindAddr` (e.g. `127.0.0.1:8420` vs `0.0.0.0:8420`), `apiKey`, `publicBaseUrl`. **Read responses redact `apiKey` to `"[REDACTED]"`** so the bearer token isn't echoed back on every overview; writes still flow through. | Network exposure, requires app restart |
| `acp_control` | `enabled`, `backends` (each: `id`, `name`, `binary`, `acpArgs`, `enabled`, `defaultModel`, `env`), `maxConcurrentSessions`, `defaultTimeoutSecs`, `runtimeTtlSecs`, `autoDiscover`. **Read responses redact non-empty `backends[*].env` to `"[REDACTED]"`** because env frequently carries `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` overrides. | Controls external agent delegation |
| `skill_env` | Per-skill env vars (may contain secrets) | Stored plaintext in `config.json` |
| `security.ssrf` | `defaultPolicy` (`strict`/`default`/`allowPrivate`), `trustedHosts` (array), per-tool overrides `browserPolicy` / `webFetchPolicy` / `imageGeneratePolicy` / `urlPreviewPolicy` | Controls whether tools can reach private networks / cloud metadata. Relaxing policy or adding untrusted hosts enables SSRF attack paths |
| `security` | `skipAllApprovals` (bool) | ⚠️ **DANGEROUS MODE** — globally bypasses every tool approval gate (exec / write / edit / apply_patch / channel tools / browser / canvas). Overrides all per-session and per-channel auto-approve settings. Plan Mode restrictions still apply. A CLI flag `--dangerously-skip-all-approvals` can set this ephemerally without touching config; this field is the *persisted* switch. Treat with extreme caution and confirm twice |
| `smart_mode` | `strategy` (`self_confidence` / `judge_model` / `both`), `judgeModel.{providerId, model, extraPrompt}` (required when strategy ∈ {judge_model, both}), `fallback` (`default` / `ask` / `allow`) | Reshapes which tool calls auto-approve in any session running `permission_mode = smart`. `judge_model` / `both` issue an extra side_query (5s hard timeout, 60s TTL) per approvable call — picking a slow / expensive model affects cost and latency across the board. `fallback: "allow"` can silently approve tools when the judge is unreachable. |
| `mcp_global` | `enabled`, `maxConcurrentCalls`, `backoffInitialSecs`, `backoffMaxSecs`, `consecutiveFailureCircuitBreaker`, `autoReconnectAfterCircuitSecs`, `deniedServers` (array of server-name strings) | MCP subsystem kill switch + concurrency caps + reconnect/backoff tuning + enterprise deny-list. Flipping `enabled=false` short-circuits every dispatch on next call (existing sessions stay open until they idle out); `deniedServers` additions prevent users from adding specific server names; loosening the backoff / circuit-breaker settings can cause aggressive retry storms against an upstream server. `alwaysLoad` is a per-server attribute on `mcp_servers`, not a `mcp_global` field. |

### Read-only (cannot be modified via this tool)

| Category | Description |
|----------|-------------|
| `active_model` | Current primary model — use Settings UI |
| `fallback_models` | Fallback chain — use Settings UI |
| `channels` | IM Channel accounts (Telegram / WeChat / Feishu / QQ / Discord). Read returns the account list with **`credentials` and `settings` fields redacted** (`"[REDACTED]"`); structural metadata (`id`, `channelId`, `label`, `enabled`, `agentId`, `autoApproveTools`, `security`) is exposed so the model can reference accounts without seeing bot tokens. Writes must go through Settings → Channels so the registry can drop/re-establish listeners under user supervision and credentials stay out of conversation logs. |
| `mcp_servers` | MCP server configs. Read returns the server list with **`env`, `headers`, `oauth` fields redacted**. Writes must go through Settings → MCP Servers UI which enforces "trust acknowledgement" for stdio servers and routes credentials through `platform::write_secure_file` (0600). |

Model / Provider / API Key / IM Channel accounts / MCP server configs / per-session configs require the Settings UI.

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
- **Field names are camelCase** (e.g. `softRatio`, `toolTimeout`, `askUserQuestionTimeoutSecs`).
- **Security restrictions** — cannot modify Providers or API Keys through this tool; guide the user to the Settings UI.
- **Surface side effects** — if the response has `sideEffect` (e.g. "requires restart"), tell the user.
- **Secrets in logs** — never echo `apiKey`, `remoteApiKey`, or `skill_env` values back in chat unless the user explicitly asks. Note that `get_settings` for `server` / `web_search` / `image_generate` / `acp_control` already redacts the credential fields to `"[REDACTED]"` — if you see that marker, the field is set but the value is hidden from the model intentionally.
- **Rollback is built-in** — if a change goes wrong, offer `restore_settings_backup` instead of trying to reconstruct the old values manually.
