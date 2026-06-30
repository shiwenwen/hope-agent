---
name: ha-debug
description: "Hope-native debugging workflow for failing tests, runtime errors, crashes, regressions, flaky behavior, bad output, or confusing logs. Use when the user asks to debug, diagnose, reproduce, trace, root-cause, or fix a bug. Emphasizes observe-before-change, smallest credible hypothesis, minimal fix, and targeted regression verification. Chinese triggers: debug, 排查, 复现, 定位问题, 崩溃, 报错, 失败测试."
---

# Hope Debug

Debug from evidence. Avoid guessing fixes into the codebase.

## Workflow

1. Reproduce or characterize the failure from the user's report, logs, tests, or current behavior.
2. Identify the smallest subsystem boundary where the failure can originate.
3. Inspect recent diffs, call paths, config, persistence, and platform assumptions.
4. Form one or two concrete hypotheses, then gather evidence for them.
5. Patch the smallest root cause fix.
6. Run a targeted regression check that exercises the failing path.

## Evidence Sources

Use whichever are available and relevant:

- Failing command output or stack traces.
- Logs, session state, database rows, or persisted job state.
- Recent git diff and commit history.
- Architecture docs for subsystem invariants.
- Existing tests or fixtures that already encode the expected behavior.

## Guardrails

- Do not rewrite a subsystem before proving the fault.
- Do not broaden scope just because nearby code is imperfect.
- Do not mark a bug fixed without a targeted check or a clear reason verification cannot run.
- When a failure depends on credentials, external services, or user state, ask for the missing evidence instead of inventing it.

## Verification

Choose the narrowest check that would have failed before the fix. For this repo, honor `AGENTS.md`: prefer `cargo check -p <crate>` or frontend typecheck during development, and ask before running full clippy/test/lint suites unless the change is a broad stage closeout.

## Smoke Prompts

- "This command fails; debug and fix it."
- "The app crashes when I do X; find the root cause."
- "A previous change regressed behavior Y; make the smallest safe fix."
