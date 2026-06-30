---
name: ha-code-review
description: "Hope-native code review procedure for staged, unstaged, untracked, branch, commit, or PR diffs. Use when the user asks to review code, inspect uncommitted changes, check a commit, find regressions, or provide actionable review comments. Outputs findings first, focuses on correctness/security/performance/maintainability, and respects inline comment formatting when requested. Chinese triggers: code review, 代码审查, 检查更改, review 当前改动, 复核 commit."
---

# Hope Code Review

Review as a maintainer, not as a summarizer.

## Review Workflow

1. Establish the review target: uncommitted diff, staged diff, commit range, branch diff, or PR context.
2. Read the changed files and enough surrounding code to understand behavior.
3. Look for issues introduced by the change, not old unrelated problems.
4. Prefer no finding over speculative feedback.
5. When a finding is real, make it discrete, actionable, and tied to a file/line or function.

## Finding Bar

Call out an issue only when it meaningfully affects one of:

- Correctness or data loss.
- Security or privacy.
- Performance that matters in realistic usage.
- Maintainability of a shared contract.
- Missing verification for behavior that is easy to regress.

Do not report style preferences, naming nits, broad rewrites, or "could be cleaner" comments unless they hide a concrete risk.

## Output Rules

- Lead with findings ordered by severity.
- Include file and line references when available.
- Keep each finding concise: scenario, impact, fix direction.
- If inline review directives are requested, emit one directive per actionable changed-line finding.
- If there are no actionable issues, say that directly and mention residual test risk only if relevant.

## Verification

Do not run broad test suites just to review unless the user asks. Use targeted commands only when they are cheap and materially improve confidence.

## Smoke Prompts

- "Review my uncommitted changes."
- "Check this commit for correctness regressions."
- "Review the current diff and leave inline comments only for actionable issues."
