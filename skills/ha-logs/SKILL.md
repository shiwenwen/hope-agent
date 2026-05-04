---
name: ha-logs
description: "Self-service diagnostics — query Hope Agent's local SQLite databases (logs / sessions / async jobs) directly via the `exec` tool to investigate problems, analyze usage, and locate root causes. Trigger on: user reports something broken / failing / slow / stuck / not responding ('X 不工作', 'X 报错', 'X 卡住', '为什么 X 失败', 'why did X fail', 'show me the logs', 'check what happened'); ad-hoc data analysis ('this week's token usage', '最近调用最多的工具', 'how many subagent runs failed', 'tool error rate', 'find sessions where X happened'); verifying a fix ('did the error stop after I changed Y'). Use BEFORE asking the user to paste log snippets — the data is on disk, query it directly. Read-only — SELECT only, never UPDATE/DELETE/INSERT/DROP."
version: 1.0.0
author: Hope Agent
license: MIT
requires:
  anyBins: [sqlite3, python3]
---

# Hope Agent Logs — Self-Service Diagnostics

Hope Agent persists every log line, every session message, and every async tool job into local SQLite databases under `~/.hope-agent/`. You can query these directly via `exec` to investigate problems before asking the user. Treat this as your primary evidence source — `AGENTS.md` explicitly designates the log database as "the agent's first source for self-repair".

## Iron rule: read-only

**SELECT only. Never UPDATE / DELETE / INSERT / DROP / ATTACH / VACUUM / CREATE / REPLACE.**

The DBs are live: write queries can corrupt session state, kill running streams, or wipe history. Open with `-readonly` (CLI) or `?mode=ro` (Python URI) so SQLite enforces it. If you genuinely need to modify state, use the dedicated tools (`update_settings`, `task_update`, etc.) or ask the user.

## How to query

Use `exec` to run one of:

### `sqlite3` CLI (Linux / macOS, usually preinstalled)

```bash
sqlite3 -readonly -cmd ".mode column" -cmd ".headers on" ~/.hope-agent/logs.db \
  "SELECT timestamp, level, category, source, message
     FROM logs
    WHERE level = 'ERROR'
    ORDER BY timestamp DESC
    LIMIT 20;"
```

### Python fallback (Windows or no `sqlite3` on PATH)

```bash
python3 - <<'PY'
import sqlite3, os
p = os.path.expanduser("~/.hope-agent/logs.db")
con = sqlite3.connect(f"file:{p}?mode=ro", uri=True)
for r in con.execute("""
    SELECT timestamp, level, category, source, message
      FROM logs
     WHERE level = 'ERROR'
     ORDER BY timestamp DESC
     LIMIT 20
"""):
    print(r)
PY
```

### Schema discovery

```bash
sqlite3 -readonly ~/.hope-agent/logs.db ".schema logs"
sqlite3 -readonly ~/.hope-agent/sessions.db ".tables"
```

## Databases

| Path | Purpose |
|------|---------|
| `~/.hope-agent/logs.db` | App logs from `app_info!`/`warn!`/`error!`/`debug!` macros |
| `~/.hope-agent/sessions.db` | Sessions, messages, tasks, subagent runs, learning events, channel conversations |
| `~/.hope-agent/async_jobs.db` | Async tool job spool (`exec` / `web_search` / `image_generate`) |
| `~/.hope-agent/recap/recap.db` | Cached recap analysis |
| `~/.hope-agent/local_model_jobs.db` | Local LLM background jobs (download / preload) |

## Key schemas

### `logs.db` → `logs`

| Column | Type | Notes |
|--------|------|-------|
| `id` | INTEGER PK | |
| `timestamp` | TEXT | ISO 8601 UTC, e.g. `2026-05-04T12:34:56.789Z` |
| `level` | TEXT | `INFO` / `WARN` / `ERROR` / `DEBUG` |
| `category` | TEXT | Subsystem tag — examples: `chat_engine`, `permission`, `mcp`, `channel`, `compact`, `failover`, `tool`, `provider`, `memory`, `cron`, `subagent`, `plan`, `config`, `skill` |
| `source` | TEXT | Origin file/function (free-form, agent-set) |
| `message` | TEXT | Rendered printf-style message |
| `details` | TEXT | Optional JSON payload |
| `session_id` | TEXT | Nullable, links to `sessions.db → sessions.id` |
| `agent_id` | TEXT | Nullable |

Indexes: `timestamp DESC`, `level`, `category`, `session_id`.

### `sessions.db` → `sessions`

`id, title, agent_id, provider_id, provider_name, model_id, reasoning_effort, created_at, updated_at, context_json, last_read_message_id, is_cron, parent_session_id, incognito, title_source`

### `sessions.db` → `messages`

`id, session_id, role, content, timestamp, attachments_meta, model, tokens_in, tokens_out, reasoning_effort, tool_call_id, tool_name, tool_arguments, tool_result, tool_duration_ms, is_error, ttft_ms, tokens_in_last, tokens_cache_creation, tokens_cache_read, tool_metadata, thinking`

`role` ∈ `user` / `assistant` / `system` / `tool`. Tool calls land as paired rows: an `assistant` row with `tool_name`/`tool_arguments`, then a `tool` row with `tool_result` / `is_error` / `tool_duration_ms`.

`messages_fts` (FTS5 virtual table) provides full-text search over `content` for `user`/`assistant` rows — use it for keyword search:

```sql
SELECT m.id, m.session_id, m.role, snippet(messages_fts, 0, '<mark>', '</mark>', '…', 16)
  FROM messages_fts
  JOIN messages m ON m.id = messages_fts.rowid
 WHERE messages_fts MATCH 'timeout OR rate_limit'
 ORDER BY m.timestamp DESC
 LIMIT 20;
```

### `sessions.db` → `subagent_runs`

`run_id, parent_session_id, parent_agent_id, child_agent_id, child_session_id, task, status, result, error, depth, model_used, started_at, finished_at, duration_ms, label, attachment_count, input_tokens, output_tokens`

`status` ∈ `spawning` / `running` / `completed` / `failed` / `cancelled`.

### `sessions.db` → `tasks`

Plan Mode task tracking — `session_id` + state columns. **Read only**; writes must go through `task_create` / `task_update` tools.

### `sessions.db` → `learning_events`

`id, ts INTEGER (unix seconds), kind, session_id, ref_id, meta_json` — current `kind` values: skill CRUD, `tool_recall_memory` hits, MCP tool calls. `meta_json` is opaque JSON.

### `async_jobs.db` → `async_tool_jobs`

`job_id, session_id, agent_id, tool_name, tool_call_id, args_json, status, result_preview, result_path, error, created_at INTEGER (unix s), completed_at, injected, origin`

`status` ∈ `pending` / `running` / `completed` / `failed` / `cancelled`. Large results spill to `result_path` on disk; `result_preview` keeps a short head/tail.

## Query cookbook

Pick a template, adapt the time window / filter / limit. Always bound by time and `LIMIT` — these tables can hold millions of rows.

### Recent errors (last 30 min)

```sql
SELECT timestamp, category, source, message, details
  FROM logs
 WHERE level IN ('ERROR','WARN')
   AND timestamp >= datetime('now','-30 minutes')
 ORDER BY timestamp DESC
 LIMIT 50;
```

### Errors for one session

```sql
SELECT timestamp, level, category, message, details
  FROM logs
 WHERE session_id = ?
 ORDER BY timestamp DESC
 LIMIT 100;
```

### Top error categories (last 7 days)

```sql
SELECT category, source, COUNT(*) AS n
  FROM logs
 WHERE level = 'ERROR'
   AND timestamp >= datetime('now','-7 days')
 GROUP BY category, source
 ORDER BY n DESC
 LIMIT 20;
```

### Failover events

```sql
SELECT timestamp, source, message, details
  FROM logs
 WHERE category = 'failover'
   AND timestamp >= datetime('now','-1 day')
 ORDER BY timestamp DESC;
```

### Slowest tool calls (last 24h)

```sql
SELECT tool_name,
       COUNT(*)              AS calls,
       AVG(tool_duration_ms) AS avg_ms,
       MAX(tool_duration_ms) AS max_ms
  FROM messages
 WHERE role = 'tool'
   AND tool_duration_ms IS NOT NULL
   AND timestamp >= datetime('now','-1 day')
 GROUP BY tool_name
 ORDER BY avg_ms DESC;
```

### Tool failure rate (last 7 days)

```sql
SELECT tool_name,
       SUM(is_error) AS errors,
       COUNT(*)      AS total,
       ROUND(100.0 * SUM(is_error) / COUNT(*), 1) AS error_pct
  FROM messages
 WHERE role = 'tool'
   AND timestamp >= datetime('now','-7 days')
 GROUP BY tool_name
HAVING errors > 0
 ORDER BY error_pct DESC;
```

### Token usage by day (last 30 days)

```sql
SELECT date(timestamp)         AS day,
       SUM(tokens_in)          AS in_tok,
       SUM(tokens_out)         AS out_tok,
       SUM(tokens_cache_read)  AS cache_read,
       SUM(tokens_cache_creation) AS cache_create
  FROM messages
 WHERE role = 'assistant'
   AND timestamp >= datetime('now','-30 days')
 GROUP BY day
 ORDER BY day DESC;
```

### Subagent runs failing recently

```sql
SELECT child_agent_id, label, error, started_at, duration_ms
  FROM subagent_runs
 WHERE status = 'failed'
   AND started_at >= datetime('now','-7 days')
 ORDER BY started_at DESC;
```

### Async jobs stuck / orphaned

```sql
SELECT job_id, session_id, tool_name, status, created_at, error
  FROM async_tool_jobs
 WHERE status IN ('pending','running')
   AND created_at < strftime('%s','now') - 3600   -- older than 1 hour
 ORDER BY created_at;
```

### Search log messages by keyword

```sql
SELECT timestamp, level, category, source, message
  FROM logs
 WHERE message LIKE '%timeout%'
   AND level IN ('ERROR','WARN')
 ORDER BY timestamp DESC
 LIMIT 30;
```

### Most-active sessions in the last 24h

```sql
SELECT s.id, s.title, s.agent_id, s.model_id,
       COUNT(m.id) AS msg_count,
       MAX(m.timestamp) AS last_msg
  FROM sessions s
  JOIN messages m ON m.session_id = s.id
 WHERE m.timestamp >= datetime('now','-1 day')
 GROUP BY s.id
 ORDER BY msg_count DESC
 LIMIT 10;
```

### Cross-DB: link a log row back to its conversation

```bash
# Step 1 — pick a log
sqlite3 -readonly ~/.hope-agent/logs.db \
  "SELECT id, session_id, message FROM logs WHERE id = 12345;"

# Step 2 — pull session metadata + recent messages
sqlite3 -readonly -cmd ".mode column" -cmd ".headers on" ~/.hope-agent/sessions.db \
  "SELECT id, title, agent_id, model_id FROM sessions WHERE id = '<session_id>';
   SELECT role, timestamp, substr(content,1,200), tool_name, is_error
     FROM messages WHERE session_id = '<session_id>'
     ORDER BY id DESC LIMIT 10;"
```

(Run as two separate queries — the read-only mode forbids `ATTACH`, and we don't want to modify state anyway.)

## Workflow when the user reports a problem

1. **Don't ask for log paste — query directly.** Say "Looking at the logs now" and run a query.
2. **Time-bound the scope.** "Just now" → last 30 min. "Today" → last 24 h. "This week" → 7 days.
3. **Start broad, narrow down.** First `level='ERROR'` over the window, then drill into the suspect `category` with full `details`.
4. **Cross-reference `messages.tool_result`** for tool-related issues — the log line names the failure, the message row holds the actual payload (often a stack trace or HTTP body).
5. **Report findings as an evidence chain.** Quote real timestamps and verbatim message text. Don't paraphrase vaguely.
6. **Combine with `systematic-debugging`** for non-trivial bugs — root-cause discipline beats guessing.
7. **Verify the fix.** After changing config / code, re-run the same query to confirm the error stops appearing.

## Privacy & safety caveats

- **Incognito sessions** (`sessions.incognito = 1`) deliberately don't persist user content. If a query returns thin data for an incognito session, just note it — don't treat the gap as a bug.
- **Secrets in logs**: API keys / tokens are redacted at the `app_*!` macro layer (`redact_sensitive` in [`crates/ha-core/src/logging.rs`](../../crates/ha-core/src/logging.rs)). However `messages.tool_arguments` / `tool_result` may contain user-pasted secrets — never echo raw tool arguments back in chat without scanning first.
- **Local only**: these DBs never leave the user's machine. They're private by construction; treat them that way.
- **Row counts**: `messages` and `logs` can grow to millions of rows. Always `LIMIT` and time-bound. Never `SELECT *` without filters.

## When NOT to use this skill

- For **session message replay with attachments resolved**, prefer the `sessions_history` / `session_status` tools — they handle media and pagination cleanly.
- For **listing sessions by simple filter**, prefer `sessions_list` — it already understands `agent_id` / `is_cron` / pagination.
- For **dashboard aggregates** (cost trends, heatmap, health score), the GUI Dashboard surface already runs these — query logs only for ad-hoc / unsupported angles.
- For **mutating state**, use the corresponding write tool (`update_settings`, `task_update`, `task_create`, etc.). Never write SQL.

## If neither `sqlite3` nor `python3` is available

Tell the user:

> I'd query the logs directly but neither `sqlite3` nor `python3` is on PATH. Could you (a) install `sqlite3` (`brew install sqlite` on macOS, `apt install sqlite3` on Debian/Ubuntu), or (b) paste the relevant lines from `~/.hope-agent/logs.db` so I can inspect manually?
