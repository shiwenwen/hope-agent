---
name: ha-coding-common
description: "Hope-native baseline for implementing, fixing, refactoring, and maintaining code: inspect the repository first, protect user changes, keep scope narrow, and finish with direct evidence."
paths: ["*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.py", "*.go", "*.java", "*.kt", "*.swift", "*.c", "*.cpp", "*.h", "*.rb", "*.php", "*.sh"]
---

# Hope Coding Common

Use this as the default discipline for coding work. Load a narrower `ha-*`
coding skill when planning, debugging, testing, review, multi-agent execution,
verification, or Workflow authoring is the real center of the task.

## Precedence

1. Follow the user's current request.
2. Read and follow the nearest `AGENTS.md` and repository instructions.
3. Preserve existing architecture and local conventions unless the task requires
   changing them.
4. Treat this skill as methodology only. It never grants permissions, changes a
   mode, or overrides a runtime safety gate.

## Start From Evidence

- Inspect the worktree, relevant files, nearby tests, and existing docs before
  choosing an implementation.
- Search with `rg` / `rg --files` when available; read surrounding code rather
  than editing from a single match.
- Assume uncommitted changes may belong to the user. Never revert, overwrite, or
  reformat unrelated work.
- Prefer existing helpers, error types, state models, and ownership boundaries.

## Size The Work

### Small and clear

Act directly when the behavior, owning file, and verification path are obvious.
Do not create a formal plan, new abstraction, or subagent ceremony merely because
the task is coding.

### Multi-step or uncertain

Use `ha-coding-plan` when the change spans ownership boundaries, has ordering
constraints, carries migration risk, or needs explicit completion criteria. In
normal execution mode, continue implementing after the plan when the next action
is clear. Plan Mode remains read-only.

### Specialized work

- Bug, regression, crash, or failing test: `ha-debug`.
- Test design or regression coverage: `ha-test-strategy`.
- Review request: `ha-code-review`.
- Independent fan-out with meaningful parallel benefit: `ha-multi-agent-coding`.
- Proof of completion: `ha-verify`.
- Durable `workflow.js`: `ha-workflow-script`.

Load the smallest useful set. Do not activate every coding skill up front.

## Control-Plane Boundaries

- Goal defines the durable outcome and completion criteria.
- Plan describes an implementation approach; it does not create a Goal.
- Task exposes current progress and must reflect actual state.
- Workflow executes one durable, observable orchestration run.
- Loop decides when another turn should be triggered.
- Worktree isolates writes; it is not a planning or completion signal.

Do not silently enable or complete any control plane from skill instructions.

## Change Discipline

- Keep edits scoped to the requested behavior and owning subsystem.
- Add an abstraction only when it removes real complexity or matches a local
  pattern.
- Use structured parsers and APIs for structured data.
- Avoid unrelated cleanup, metadata churn, generated files, and speculative
  compatibility layers.
- For multi-step work, keep user-visible tasks truthful and only one task in
  progress unless the runtime is genuinely executing independent work.
- Ask only when the next step is unsafe, irreversible, or cannot be inferred
  from available evidence.

## Finish The Work

1. Inspect the final diff and current worktree state.
2. Use `ha-verify` to map requirements to the smallest sufficient evidence.
3. Report what changed, what was verified, and any real residual risk.
4. Do not claim completion from intent, a passing unrelated command, or child
   Agent completion alone.

## Smoke Prompts

- "Implement this small feature and keep the diff minimal."
- "Finish this refactor without touching unrelated user changes."
- "Continue the current coding task through targeted verification."
