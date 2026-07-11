---
name: ha-code-review
description: "Hope-native review of uncommitted, staged, commit, branch, or PR changes: discover concrete regressions, independently verify candidates, and report actionable findings first without speculative noise."
paths: ["*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.py", "*.go", "*.java", "*.kt", "*.swift", "*.c", "*.cpp", "*.h", "*.rb", "*.php", "*.sh"]
---

# Hope Code Review

Review as a maintainer. The default action is to inspect and report, not to edit.
Only repair findings when the user asks for fixes.

## Establish The Target

Identify exactly what is under review:

- Staged, unstaged, and untracked changes.
- A commit or commit range.
- Branch or PR diff against the correct base.
- A named file or subsystem.

Read the changed files and enough surrounding code, tests, architecture, and
callers to understand behavior. Do not report unrelated pre-existing issues as
findings introduced by the change.

## Two-Phase Reasoning

### Discovery

Search from multiple relevant angles:

- Correctness, state transitions, error paths, and data loss.
- Security, privacy, permissions, and unsafe trust boundaries.
- Concurrency, cancellation, retry, persistence, and recovery.
- Performance on realistic hot paths.
- Cross-module contracts, compatibility, and missing regression coverage.

Generate candidate issues without committing to them.

### Verification

For each candidate:

1. Trace a concrete scenario that reaches the changed behavior.
2. Confirm the issue was introduced or exposed by the review target.
3. Check whether surrounding guards or tests already prevent it.
4. Keep it only if the author would likely fix it once informed.

Prefer no finding over a speculative or stylistic finding.

For a small, low-risk diff, one reviewer can perform both phases. For a broad or
high-risk diff, an independent read-only reviewer may verify candidates. Do not
mandate a fixed number of Agents or two review passes for every change.

## Finding Bar

A finding must be discrete, actionable, and materially affect correctness,
security, privacy, performance, maintainability of a shared contract, or
regression protection. It must explain the triggering scenario and impact.

Do not report:

- Personal naming or formatting preferences.
- Broad rewrites without a concrete failure.
- Problems outside the changed behavior.
- Test requests that do not protect a meaningful contract.

## Output

- Lead with findings ordered by severity.
- Use the smallest useful file/line or function reference.
- State scenario, impact, and fix direction concisely.
- Emit inline review directives only when requested and only for actionable
  changed-line findings.
- If there are no actionable findings, say so directly and mention only real
  residual verification gaps.

Do not bury findings under a summary. Do not modify code in review-only mode.

## Verification Cost

Use cheap targeted checks only when they materially improve confidence. Follow
repository instructions and do not run broad suites solely to make the review
look thorough.

## Smoke Prompts

- "Review all my uncommitted changes."
- "Check this commit for behavioral regressions."
- "Review this PR and leave inline comments only for real issues."
