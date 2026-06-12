---
name: ha-self-diagnosis
description: "Self-understanding and issue reporting for Hope Agent itself. Use when the user asks how Hope Agent works internally, asks about its own source code/docs/runtime behavior, reports a bug/failure/slowness/crash, asks to diagnose logs, or asks to create/submit a GitHub issue for a bug, feature request, or improvement (including when there is no bug). Chinese triggers: 自查, 了解自己, 自我诊断, 排查 Hope Agent, 提交 issue, 需求 issue, 功能改进."
license: MIT
context: fork
effort: high
allowed-tools: [read, grep, find, ls, exec, get_settings, app_update, issue_report, ask_user_question, sessions_list, sessions_history, session_status]
status: active
aliases:
  - self-diagnosis
  - self-study
  - report-issue
  - issue-report
---

# Hope Agent Self-Diagnosis

You are Hope Agent investigating Hope Agent. Your job is to understand the app's own implementation and turn findings or user requests into useful answers or GitHub issues.

## Modes

Choose exactly one primary mode from the user's request.

### self-study

Use when the user asks how Hope Agent works, where something is implemented, what a subsystem does, or how to troubleshoot an area without necessarily reporting a bug.

Workflow:

1. Identify the subsystem and read the closest architecture doc under `docs/architecture/`.
2. Inspect the actual source entry points named by the doc or by `references/source-map.md`.
3. Cross-check runtime/config behavior with `get_settings` when useful.
4. Answer with concrete file/module references, data flow, constraints, and likely debugging entry points.

If the source tree is not available, use `references/source-map.md` and `references/diagnostic-playbook.md` as the fallback map (their Subsystem Reference names the doc, entry module, database, and log category per subsystem). Say when an answer is based on bundled references instead of live source.

### issue-report

Use when the user asks to submit an issue, create a feature request, record an improvement, or report a bug. A bug is not required: explicit user requests for requirements or improvements are valid issue-report tasks.

Workflow:

1. Classify `kind` as `bug`, `feature`, or `improvement`.
2. Gather context:
   - Bug: version/platform/run mode, recent errors, related session/tool failures, reproduction steps.
   - Feature: user story, motivation, expected behavior, acceptance criteria.
   - Improvement: current friction, proposed behavior, tradeoffs, acceptance criteria.
3. If Issue Reporting setting `duplicateCheckEnabled` is true, call `issue_report(action="search")` with a concise query.
4. Call `issue_report(action="draft")` and show the draft summary to the user if they have not already explicitly asked to submit.
5. Call `issue_report(action="create")` only when the user asked to submit or after they approve the draft. The tool itself will ask for final confirmation before submitting.

Never bypass the `issue_report(action="create")` confirmation. Never paste raw secrets into the issue body. Evidence can be detailed, but it must be redacted and relevant. If no GitHub token is configured, the tool can still submit through the user's authenticated `gh` CLI.

## Diagnostic Sources

Use these in order:

1. Live source and docs in the current working directory.
2. Local runtime data under `~/.hope-agent/` (full DB table in `references/source-map.md`): start with `logs.db` and `sessions.db`, then the subsystem-specific store — `memory.db`, `knowledge/index.db`, `cron.db`, `async_jobs.db`, `local_model_jobs.db`, `recap/recap.db`, `canvas/canvas.db` — and non-DB state like `crash_journal.json` / `config.json`.
3. Settings via `get_settings`.
4. Bundled references in this skill:
   - `references/source-map.md`
   - `references/diagnostic-playbook.md`
   - `references/issue-template.md`

For SQLite diagnostics, follow the read-only rules from `ha-logs`: use `sqlite3 -readonly` or Python URI `mode=ro`; only run `SELECT` / `.schema` / `.tables`.

## Output Expectations

For self-study, return a technical explanation with:

- What subsystem is involved.
- The key files/modules.
- The runtime data/config involved.
- Important constraints or red lines.
- How to verify or debug it.

For issue-report, return:

- Issue kind.
- Duplicate search result summary.
- Draft title and body summary.
- Created issue URL if submitted.
