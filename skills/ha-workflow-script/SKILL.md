---
name: ha-workflow-script
description: "Hope-native authoring and review for durable workflow.js runs: deterministic host APIs, typed child results, bounded parallel/pipeline execution, budgets, replay-safe identity, staged consumption, and honest closure."
paths: ["*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.py", "*.go", "*.java", "*.kt", "*.swift", "*.c", "*.cpp", "*.h", "*.rb", "*.php", "*.sh"]
---

# Hope Workflow Script

Use this for script-first dynamic Workflow authoring or repair. Workflow Mode
lets the model decide whether orchestration helps; the user should not need to
write `workflow.js` or enter a special coding mode.

## Boundary

- A Workflow is one durable, observable execution run.
- A Goal owns the durable outcome and completion criteria.
- A Loop schedules another trigger; do not implement recurrence inside a
  Workflow script.
- Task handles expose progress; they are not op identities.
- Permissions, approvals, isolation, quotas, and closure gates are enforced by
  the runtime, not by this text.

## Deterministic Runtime

- Export `default async function main(workflow)` and finish with
  `workflow.finish(result)`.
- Runtime op identity comes from deterministic execution position. `label` is
  display-only and never an id.
- Keep the script hash fixed for a run. Edited-script resume may reuse only the
  safe matching prefix allowed by runtime provenance.
- Use `workflow.now()` and `workflow.random(seed)` instead of ambient time or
  randomness.
- No raw filesystem, process, environment, network, dynamic import, `eval`, or
  `Function`; use approved host APIs.

## Recommended Shape

1. Validate `workflow.meta`, `workflow.args`, scope, criteria, and budget.
2. Create user-visible tasks and retain returned task handles.
3. Observe current state through read/search host APIs.
4. Choose sequential, `parallel`, or `pipeline` execution based on dependency
   shape and cost.
5. Consume child results at useful checkpoints, steer or cancel when evidence
   changes, then run targeted validation.
6. Finish with result, artifacts, verification, and residual risk.

## Child Agents And Typed Results

- Use `outputSchema` when the parent needs machine-consumable fields.
- Keep `schemaRetries` bounded and reserve output tokens before spawn.
- Treat repair output as structure repair, not permission to redo or expand the
  task.
- Default write-capable work to isolated worktrees. Use
  `isolation: "shared_read_only"` only for genuinely read-only work; the runtime
  hard tool set is the security boundary.
- Child completion is an input to synthesis, not proof that the Workflow or Goal
  is complete.

## Parallelism And Stage Consumption

- `workflow.parallel(...)` fits bounded independent work followed by a barrier.
- `workflow.pipeline(...)` fits a bounded window where fast results should be
  consumed and replenished before slow children finish.
- `workflow.waitAny(...)` supports staged decisions.
- `workflow.waitAll(...)` is valid when the task truly requires a barrier; status
  mode observes without consuming output.
- `workflow.agentResult(...)` reads a child result; use `agentStatus`, steering,
  cancellation, or additional spawn when the plan must adapt.
- Check `workflow.budgetStatus()` before expanding fan-out.

Never create unbounded fan-out, recursive Workflow execution, or a fixed
wait-all policy for every task.

## Replay And Closure

- Materialize fan-out inputs and keep callback order deterministic.
- Update tasks by handle, never by label.
- Preserve typed result provenance and partial failures during synthesis.
- `workflow.finish()` cannot honestly complete while owned children remain
  non-terminal; if the runtime budget expires, return blocked rather than a
  false success.
- A completed Workflow must still provide a user-meaningful result to the main
  Agent; the completion registration itself is not the answer.

## Review Checklist

- Are all side effects behind host APIs and permission gates?
- Are identity, inputs, fan-out, time, and randomness replay-safe?
- Are isolation and output schemas appropriate?
- Can useful partial results be consumed without busy waiting?
- Are budget, validation, failure, and stop conditions explicit?
- Does the final result distinguish child completion from outcome completion?

## Smoke Prompts

- "Draft a replay-safe workflow.js with typed parallel reviewers."
- "Use pipeline consumption instead of waiting for every child."
- "Review this Workflow for budget, isolation, and closure bugs."
