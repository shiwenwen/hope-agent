# Diagnostic Playbook

## Bug or Failure

1. Capture the user's symptom in one sentence.
2. Determine run mode: desktop, server, ACP, IM channel, cron, subagent, or MCP.
3. Check settings relevant to the subsystem with `get_settings`.
4. Query recent logs read-only:

```bash
sqlite3 -readonly -cmd ".headers on" -cmd ".mode column" ~/.hope-agent/logs.db \
  "SELECT timestamp, level, category, source, message, details
     FROM logs
    WHERE level IN ('ERROR','WARN')
    ORDER BY timestamp DESC
    LIMIT 80;"
```

5. If the issue is session-specific, inspect messages/tool rows:

```bash
sqlite3 -readonly -cmd ".headers on" -cmd ".mode column" ~/.hope-agent/sessions.db \
  "SELECT timestamp, role, tool_name, is_error, substr(content,1,240) AS content,
          substr(tool_result,1,240) AS tool_result
     FROM messages
    WHERE session_id = '<SESSION_ID>'
    ORDER BY timestamp DESC
    LIMIT 80;"
```

6. Map symptoms to source files using `references/source-map.md` and `docs/architecture/`.
7. Check whether a newer release may already fix it with `app_update(action="check")` when appropriate.
8. For issue reporting, include only relevant, redacted evidence.

## Self-Study

1. Start with the architecture doc for the subsystem.
2. Read the source that implements the documented contract.
3. Prefer public interfaces, command/route adapters, config structs, and tool schemas over isolated helper details.
4. Explain what happens in desktop and server modes when the behavior crosses transport boundaries.
5. Call out red lines: config mutation helpers, SSRF checks, provider CRUD helpers, Plan Mode restrictions, and read-only DB rules.

## Feature or Improvement Issue

1. Restate the user request as a product outcome.
2. Identify affected workflows and likely modules.
3. Capture acceptance criteria.
4. Mention constraints from architecture docs.
5. Search existing issues before creating if duplicate checks are enabled.
6. Use `issue_report(action="draft")`, then `issue_report(action="create")` after the user requests submission or approves the draft.
