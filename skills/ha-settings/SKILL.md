---
name: ha-settings
description: "Manage Hope Agent application settings through conversation. Use when the user wants to view or change any app configuration: theme, language, proxy, temperature, notifications, tool timeout, context compaction, automatic session titles, web search, memory, embedding, recap, behavior awareness, plan mode, ask-user-question timeout, tool-result disk spill threshold, embedded server, ACP control plane, per-skill env vars, or any other setting visible in the Settings UI. Trigger phrases: 'change settings', 'configure proxy', 'set theme to dark', 'turn off notifications', 'adjust temperature', 'show my settings', 'bind the server to all interfaces', 'set API key'. Trigger even when the user doesn't explicitly say 'settings' — any intent to adjust app behavior qualifies."
always: true
---

# Settings — Application Configuration Management

Use `get_settings` and `update_settings` to read and modify settings. **Never edit config files directly.** Coverage matches the desktop Settings UI one-to-one (except Providers / API Keys, which stay UI-only for security).

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
| `canvas` | `enabled` |
| `image` | `maxImages` |
| `pdf` | `maxPdfs`, `maxVisionPages` |
| `image_generate` | `provider`, `model` |
| `temperature` | `temperature` (0.0–2.0, null = API default) |
| `tool_timeout` | `toolTimeout` (seconds, 0 = unlimited) |
| `default_agent` | `defaultAgentId` (string id; `null` / empty falls back to hardcoded `"default"` agent) |

### MEDIUM risk — behavioral changes (cost, context, output quality)

| Category | Fields |
|----------|--------|
| `compact` | `enabled`, `cacheTtlSecs`, thresholds |
| `session_title` | `enabled`, `providerId`, `modelId` (null provider/model = use the chat model). When enabled, new sessions keep the first-message fallback title immediately, then run one LLM call after the first assistant reply to generate a concise title. Manual renames are never overwritten. |
| `memory_extract` | `enabled`, `cooldownSecs`, `tokenThreshold` |
| `memory_selection` | `enabled`, `candidateThreshold`, `maxSelected` |
| `memory_budget` | `totalChars` (int, default 10000), `coreMemoryFileChars` (int, default 8000 — cap per `memory.md` file), `sqliteEntryMaxChars` (int, default 500 — cap per rendered SQLite bullet), `sqliteSections.{userProfile,aboutUser,preferences,projectContext,references}` (defaults 1500/2000/2000/3000/1500; `userProfile` was renamed from `aboutYou` and the system-prompt heading from `## About You` to `## User Profile` — the old `aboutYou` key is still accepted for back-compat). Priority order: Guidelines > Agent `memory.md` > Global `memory.md` > SQLite. Reducing `totalChars` may hide parts of `memory.md` from the system prompt; full content is still retrievable via `recall_memory` / `memory_get`. |
| `embedding_cache` | `enabled`, `maxEntries` |
| `dedup` | `enabled`, `threshold` |
| `hybrid_search` | `keywordWeight`, `vectorWeight` |
| `temporal_decay` | `enabled`, `halfLifeDays` |
| `mmr` | `enabled`, `lambda` |
| `recap` | `analysisAgent`, `defaultRangeDays`, `facetConcurrency` |
| `awareness` | `enabled`, `mode` (`structured`/`llm_digest`) |
| `web_fetch` | `enabled`, `maxBytes` |
| `web_search` | `provider`, `searxngUrl`, `tavilyApiKey` |
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
| `server` | `bindAddr` (e.g. `127.0.0.1:8420` vs `0.0.0.0:8420`), `apiKey` | Network exposure, requires app restart |
| `acp_control` | `enabled`, `backends`, `maxConcurrentSessions`, `defaultTimeoutSecs`, `runtimeTtlSecs`, `autoDiscover` | Controls external agent delegation |
| `skill_env` | Per-skill env vars (may contain secrets) | Stored plaintext in `config.json` |
| `security.ssrf` | `defaultPolicy` (`strict`/`default`/`allowPrivate`), `trustedHosts` (array), per-tool overrides `browserPolicy` / `webFetchPolicy` / `imageGeneratePolicy` / `urlPreviewPolicy` | Controls whether tools can reach private networks / cloud metadata. Relaxing policy or adding untrusted hosts enables SSRF attack paths |
| `security` | `skipAllApprovals` (bool) | ⚠️ **DANGEROUS MODE** — globally bypasses every tool approval gate (exec / write / edit / apply_patch / channel tools / browser / canvas). Overrides all per-session and per-channel auto-approve settings. Plan Mode restrictions still apply. A CLI flag `--dangerously-skip-all-approvals` can set this ephemerally without touching config; this field is the *persisted* switch. Treat with extreme caution and confirm twice |
| `channels` | `accounts`, `defaultAgentId`, `defaultModel` | Contains IM Channel Bot configurations (e.g., Telegram, WeChat tokens). Modifying this drops/reconnects listeners and handles sensitive bot credentials |
| `mcp_global` | `enabled`, `maxConcurrentCalls`, `backoffInitialSecs`, `backoffMaxSecs`, `consecutiveFailureCircuitBreaker`, `autoReconnectAfterCircuitSecs`, `deniedServers`, `alwaysLoadServers` | MCP subsystem kill switch + concurrency caps + reconnect/backoff tuning + enterprise deny-list. Flipping `enabled=false` disconnects every MCP server; `deniedServers` additions prevent users from adding specific server names; loosening the backoff / circuit-breaker settings can cause aggressive retry storms against an upstream server |

### Read-only (cannot be modified via this tool)

| Category | Description |
|----------|-------------|
| `active_model` | Current primary model — use Settings UI |
| `fallback_models` | Fallback chain — use Settings UI |
| `mcp_servers` | MCP server configs — use Settings → MCP Servers UI. Contains OAuth tokens, command arguments, and trust levels; writes must go through the GUI which enforces "trust acknowledgement" for stdio servers and routes credentials through `platform::write_secure_file` (0600). |

Model / Provider / API Key / IM Channel / MCP servers / per-session configs require the Settings UI.

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
- **Secrets in logs** — never echo `apiKey`, `remoteApiKey`, or `skill_env` values back in chat unless the user explicitly asks.
- **Rollback is built-in** — if a change goes wrong, offer `restore_settings_backup` instead of trying to reconstruct the old values manually.
