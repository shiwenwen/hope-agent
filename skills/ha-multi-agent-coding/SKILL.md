---
name: ha-multi-agent-coding
description: "Hope-native multi-agent coding orchestration: fan out only independent valuable work, enforce bounded scope and isolation, consume structured results progressively, steer or cancel, and keep synthesis with the main Agent."
paths: ["*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.py", "*.go", "*.java", "*.kt", "*.swift", "*.c", "*.cpp", "*.h", "*.rb", "*.php", "*.sh"]
---

# Hope Multi-Agent Coding

Use multiple Agents when parallel evidence or implementation meaningfully
outweighs coordination cost. Do not make delegation the default for complex-looking
work.

## Fan-Out Decision

Good candidates:

- Similar independent investigations across modules.
- Distinct review angles with a shared structured output.
- Independent implementations with non-overlapping ownership.
- A bounded set of alternatives that the main Agent will compare.

Keep work with one Agent when:

- The task is small or one search path is likely sufficient.
- Steps depend on prior results.
- Agents would edit the same files or shared generated state.
- A single broad investigation needs coherent context.
- Coordination, token, or merge cost exceeds expected parallel gain.

## Define Each Child Contract

Provide:

- One concrete objective and bounded scope.
- Relevant context already known by the parent.
- Allowed and forbidden actions.
- File ownership or read-only isolation.
- Required output schema, evidence, and stop condition.
- Verification expected from the child.

Do not re-delegate the entire parent assignment to one child. Children do not
own final user communication or Goal closure.

## Isolation

- Prefer `shared_read_only` for research, discovery, and verification.
- Use separate worktrees for independent writes.
- If writes cannot be isolated, serialize them or assign mutually exclusive file
  ownership.
- Permission mode, protected paths, approval surfaces, and tool restrictions
  remain runtime-enforced. A child prompt cannot grant access.

## Bounded Execution

Set explicit limits for fan-out count, depth, turns, tokens, and time. Respect
runtime queues and backpressure. Never recursively create Workflow runs or an
unbounded Agent tree.

## Progressive Control

The main Agent may choose based on task needs:

- Consume the first useful results and adapt (`waitAny` / checkpoint).
- Query status without consuming output.
- Read one structured result, then steer or cancel remaining work.
- Add a follow-up child when new evidence changes the decomposition.
- Wait for all children only when a true barrier is required.

Background work must not block the user's conversation. Use runtime completion
or checkpoint injection rather than polling loops.

## Synthesis

The main Agent must:

1. Check which children completed, failed, timed out, or returned no evidence.
2. Resolve conflicts using source evidence, not majority vote.
3. Preserve partial failures and uncertainty.
4. Integrate or review writes in the parent worktree deliberately.
5. Run parent-level verification for the combined outcome.

"All Agents completed" is orchestration state, not a user result. Do not finish
until the parent has synthesized and answered the actual task.

## Workflow Boundary

This skill decides delegation strategy. Use `ha-workflow-script` when execution
must be durable, replayable, observable, or script-controlled. Simple bounded
subagent work does not require a Workflow; a Workflow may use this strategy
without surrendering its runtime contracts.

## Smoke Prompts

- "Investigate these six independent modules in parallel and synthesize."
- "Use staged child results; do not wait for every slow reviewer."
- "Decide whether this implementation should stay single-Agent or use worktrees."
