---
name: ha-debug
description: "Hope-native debugging for code failures, regressions, crashes, flaky behavior, and bad output: reproduce or characterize, rank falsifiable hypotheses, fix the smallest root cause, and prove the failing path."
paths: ["*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.py", "*.go", "*.java", "*.kt", "*.swift", "*.c", "*.cpp", "*.h", "*.rb", "*.php", "*.sh"]
---

# Hope Debug

Debug from evidence, not from the first plausible explanation. This is a
decision process, not a mandatory four-stage ceremony.

## 1. Characterize The Failure

Capture the strongest available evidence:

- Exact symptom, expected behavior, and observed behavior.
- Reproduction steps, failing command, stack trace, log range, or persisted
  state.
- Whether it is deterministic, intermittent, platform-specific, data-specific,
  or timing-sensitive.
- Recent relevant diffs, dependency/config changes, and known-good boundary.

If reproduction is unsafe or requires unavailable credentials, characterize it
from logs, fixtures, state, and code paths. State the evidence gap explicitly.

## 2. Bound The Fault

Trace the smallest credible path through inputs, state transitions, persistence,
concurrency boundaries, and outputs. For multi-component systems, compare what
crosses each boundary rather than adding broad instrumentation everywhere.

Common high-value checks:

- Stale or duplicated persisted state.
- Error swallowing, fallback, retry, cancellation, and timeout paths.
- Async ordering, locks, process boundaries, and late results.
- Platform, locale, permission, path, and environment assumptions.
- Mismatch between source-of-truth data and UI projection.

## 3. Rank Falsifiable Hypotheses

Keep one or two active hypotheses. For each, write:

- Why it explains the evidence.
- What observation would disprove it.
- The cheapest discriminating check.

Run the discriminating check before editing when practical. If a tiny, obvious
fix is itself the cheapest safe experiment, keep it reversible and inspect the
result before broadening scope.

## 4. Fix The Root Cause

- Patch the smallest ownership boundary that restores the contract.
- Avoid subsystem rewrites before the fault is proven.
- Preserve unrelated user work and existing public behavior.
- Add defense-in-depth only when it covers a demonstrated adjacent failure, not
  as speculative cleanup.

After two failed fix attempts, stop patching variants. Re-read the original
evidence, challenge the shared assumption, and narrow the boundary again.

## 5. Prove The Failing Path

Use `ha-test-strategy` to choose the regression form and `ha-verify` to confirm
completion. Prefer a check that would have failed before the fix:

- Focused automated regression test.
- Existing failing command or deterministic fixture.
- Before/after database or log query.
- Manual reproduction when automation is not credible.

Passing compilation alone does not prove a runtime bug fixed. If the real path
cannot be exercised, report the strongest substitute and remaining uncertainty.

## Stop Conditions

Pause and ask for input only when progress requires inaccessible user state, an
external system change, destructive action, or a product decision. Do not invent
data or mark an unreproduced hypothesis as confirmed.

## Smoke Prompts

- "This test fails intermittently; find and fix the root cause."
- "The UI shows stale state after restart; diagnose the persistence path."
- "Use this session id and logs to explain the regression, then repair it."
