---
name: ha-workflow-script
description: "Hope-native guidance for drafting, reviewing, or repairing script-first dynamic workflow scripts for coding tasks. Use when the user asks for workflow.js, dynamic workflow, loop mode, durable replay, long-running coding automation, workflow host APIs, op identity, task handles, validation gates, or repair loop design. Chinese triggers: workflow, 动态工作流, loop 模式, 工作流脚本, 长任务自动化, durable replay."
---

# Hope Workflow Script

Draft workflow scripts as durable plans, not as free-form shell programs.

## Runtime Assumptions

- Script input is persisted as `workflow.js`; one run keeps a fixed `script_hash`.
- The runtime generates op identity from deterministic execution position.
- `label` is display-only and must not be used as an id.
- Host APIs use object arguments except `workflow.map(label, list, fn)`.
- No raw filesystem, network, process, environment, dynamic import, `eval`, or `Function`.
- Use `workflow.now()` and `workflow.random(seed)` instead of nondeterministic JS APIs.

## Script Shape

Prefer this high-level flow:

1. Create user-visible progress tasks and keep returned task handles.
2. Observe repository state through host APIs such as `fileSearch`, `read`, and `grep`.
3. Optionally spawn bounded read-only reviewers for independent exploration.
4. Implement through approved host tools rather than raw APIs.
5. Run targeted validation with a clear reason.
6. Return `workflow.finish(result)` with summary, changed files, verification, and residual risk.

## API Semantics

- `workflow.task.create({ title, label? })` returns a task handle.
- `workflow.task.update({ task, status, label? })` updates by handle, never by label.
- `workflow.map(label, list, fn)` must materialize its input list so replay uses the same fan-out.
- `workflow.validate({ commands, reason, label? })` defaults to targeted checks; full suites are human-gated.
- `workflow.askUser({ question, context?, label? })` must fail closed when no user can answer.

## Repair Loop Boundary

Do not put an unbounded repair loop inside the script. A script describes one durable run. If validation fails, return structured feedback; the runtime-level repair controller decides whether to start a guarded next attempt, ask the user, or stop.

## Review Checklist

- Is every side effect behind a host API?
- Are labels only labels?
- Are task updates handle-based?
- Are fan-out lists bounded and materialized?
- Is validation targeted and justified?
- Are stop conditions explicit enough to avoid infinite work?

## Smoke Prompts

- "Draft a workflow.js for fixing a bug and validating it."
- "Review this workflow script for replay safety."
- "Design a loop-mode workflow that stops after no-progress repairs."
