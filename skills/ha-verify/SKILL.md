---
name: ha-verify
description: "Hope-native verification planner for choosing and running the smallest useful checks after code or doc changes. Use when the user asks what to test, asks to verify a fix, asks whether Phase work is complete, asks before commit/push, or when an agent must decide between targeted checks and expensive full suites. Enforces AGENTS.md and avoids automatic full-suite runs. Chinese triggers: 验证, 测试什么, 跑检查, 收尾检查, 是否完成, 提交前验证."
---

# Hope Verify

Verification proves the changed behavior, not that "some command passed."

## Pick Checks

1. Read project instructions first, especially `AGENTS.md`.
2. Identify the actual requirement or risk being verified.
3. Choose the smallest command, fixture, diff inspection, or manual evidence that covers that requirement.
4. Prefer deterministic local checks over broad, slow, flaky, or unrelated suites.
5. Explain skipped checks when they would be useful but are blocked, expensive, or disallowed by instructions.

## Default Policy

- Docs-only: inspect diff and run a lightweight formatting/whitespace check if useful.
- Rust implementation: `cargo check -p <crate>` first.
- Frontend implementation: repo typecheck first.
- Runtime behavior: targeted command, fixture, or local reproduction.
- Long-running background work: rely on job completion injection; use `job_status` for snapshots, not busy waiting.

## Full Suite Gate

Do not run full clippy, all tests, full lint, or push-equivalent gates automatically unless:

- The user explicitly asks.
- The repo instructions require it for this stage.
- A broad multi-module closeout justifies it and you announce why first.

If the user asks to push, rely on the repo's pre-push hook instead of duplicating the full hook manually unless they ask for a local dry run.

## Completion Audit

When asked whether work is complete:

- Turn the requirement into concrete evidence items.
- Inspect current files, commands, tests, docs, and runtime state as needed.
- Treat weak or indirect evidence as incomplete.
- State exactly what is proven and what remains unverified.

## Smoke Prompts

- "What should we run to verify this change?"
- "Check whether Phase 2.1 is actually complete."
- "Before commit, do the right minimal verification."
