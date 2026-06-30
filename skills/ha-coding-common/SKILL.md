---
name: ha-coding-common
description: "Shared Hope-native coding discipline for implementation, bug fixing, refactoring, review follow-up, and repository maintenance. Use for general coding tasks when no narrower ha-* coding skill is a better fit, especially when the agent must inspect the current repo, respect AGENTS.md, keep changes scoped, track progress with tasks, avoid reverting user work, and choose targeted verification. Chinese triggers: 编码, 实现, 修复, 改代码, 优化, 提交前整理."
---

# Hope Coding Common

Use this skill as the baseline behavior for coding work.

## Operating Rules

- Read the nearest `AGENTS.md` and existing code before choosing an approach.
- Treat the current worktree as shared with the user. Do not revert or overwrite changes you did not make.
- Prefer `rg` / `rg --files` for search. Read surrounding code before editing.
- Keep edits scoped to the requested behavior and the owning subsystem.
- For multi-step work, create or update tasks; keep only one task in progress.
- Use existing project patterns, helpers, error types, config plumbing, and tests before adding new abstractions.
- Ask the user only when the next step is genuinely unsafe or cannot be inferred from local evidence.

## Change Discipline

Before editing:

1. Identify the behavioral surface being changed.
2. Check related architecture or roadmap docs when the subsystem has them.
3. List the smallest files that need changes.

While editing:

- Prefer narrow patches.
- Preserve unrelated formatting and metadata.
- Add comments only when they clarify non-obvious constraints.
- Do not add broad refactors just because the area looks messy.

After editing:

- Inspect the diff.
- Run the smallest meaningful verification allowed by project instructions.
- If verification is skipped, say why.

## Verification Defaults

- Rust code: prefer `cargo check -p <crate>` unless the repo instructions say otherwise.
- TypeScript / React: prefer the repo's typecheck command.
- Docs-only changes: no test command is usually needed; still run a lightweight diff/format sanity check when useful.
- Full lint/test suites are stage gates, not the default during development. Run them only when the user asks, the repo instructions require it, or the change is broad enough to justify the cost.

## Smoke Prompts

- "Implement this small feature and keep the diff minimal."
- "Fix this bug without touching unrelated files."
- "Clean up the uncommitted changes and prepare a concise handoff."
