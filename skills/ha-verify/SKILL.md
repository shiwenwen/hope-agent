---
name: ha-verify
description: "Hope-native completion and verification discipline: map each requirement to current direct evidence, choose the smallest sufficient checks, and distinguish proven, failed, blocked, stale, or unverified claims."
paths: ["*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.py", "*.go", "*.java", "*.kt", "*.swift", "*.c", "*.cpp", "*.h", "*.rb", "*.php", "*.sh"]
---

# Hope Verify

Verification proves the requested behavior. It is not a list of commands and it
does not decide the development order; use `ha-test-strategy` for that.

## Build An Evidence Matrix

For each explicit requirement or completion criterion, record:

| Requirement | Direct evidence | Status | Gap |
|---|---|---|---|
| Expected behavior | test, runtime observation, diff, artifact, or read-back | proven / failed / blocked / unverified | next useful check |

Evidence must be current, attributable to the changed state, and strong enough
for the requirement. A compile check cannot prove a UI interaction; a child
Agent finishing cannot prove the parent outcome; a generated artifact cannot
prove delivery without read-back.

## Choose The Smallest Sufficient Check

Follow repository instructions first. Typical order:

1. Inspect the final diff and state transition.
2. Run the focused unit, fixture, command, or reproduction for the changed path.
3. Add integration or E2E only when the contract crosses that boundary.
4. Use manual smoke evidence for visual or environment-dependent behavior.
5. Run full gates only when requested, required by the repository, or justified
   by a broad closeout.

For long background work, rely on completion injection and use status queries
for snapshots. Do not busy-wait.

## Interpret Results

- Confirm the command exercised the intended case, not merely that it exited 0.
- Record failures and skipped checks; do not reuse a result from before the last
  relevant edit.
- Separate product failure from fixture, environment, credential, and external
  service failure.
- Treat deterministic substitutes as substitutes, not as real external proof.
- If evidence is indirect or missing, the requirement remains unverified.

## Completion Audit

Before claiming a phase, Goal, or task complete:

1. Re-read the actual request and named plan.
2. Enumerate every required artifact, invariant, gate, and cleanup action.
3. Inspect authoritative current state for each item.
4. Confirm no required work remains and no stale status contradicts closure.
5. State what was proven, what was not run, and any residual risk.

Goal closure remains controlled by the Goal runtime and its evidence/grader
contract. This skill may gather evidence but cannot bypass acceptance or close a
Goal by assertion.

## Repository-Friendly Defaults

- Docs-only: inspect content, references, and whitespace.
- Rust: focused test or `cargo check -p <crate>` during implementation.
- Frontend: targeted component test or repository typecheck.
- Runtime behavior: fixture, local reproduction, logs, DB state, or read-back.
- Push: rely on the repository pre-push gate instead of manually duplicating it.

## Smoke Prompts

- "Prove whether this phase is actually complete."
- "Choose the right checks for this fix."
- "Audit the implementation against every roadmap item."
