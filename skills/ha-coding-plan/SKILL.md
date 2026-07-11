---
name: ha-coding-plan
description: "Hope-native implementation planning for non-trivial code changes: ground the plan in repository evidence, order dependencies, name critical files and risks, define verification, then continue execution when allowed."
paths: ["*.rs", "*.ts", "*.tsx", "*.js", "*.jsx", "*.py", "*.go", "*.java", "*.kt", "*.swift", "*.c", "*.cpp", "*.h", "*.rb", "*.php", "*.sh"]
---

# Hope Coding Plan

Plan only when planning reduces real uncertainty. A plan is an implementation
map, not a ritual and not a substitute for doing the work.

## When A Formal Plan Helps

Use a plan when the task has one or more of:

- Multiple ownership boundaries or dependent steps.
- Schema, migration, persistence, compatibility, or recovery risk.
- Ambiguous architecture with several credible approaches.
- Parallel work that needs explicit isolation and synthesis.
- Named completion criteria or a long-running Goal.

Skip a formal plan for a small, obvious, reversible edit with a direct check.

## Evidence First

Before planning:

1. Read the user request and explicit completion criteria.
2. Read `AGENTS.md`, relevant architecture, current diff, and critical code.
3. Find a similar implementation and trace the owning path.
4. Identify unknowns that materially change the design.

Do not invent files, APIs, tests, or migrations from naming alone.

## Plan Contents

Each step should name:

- Outcome and behavior changed.
- Critical files or subsystem, without pretending the exact line is known when
  it is not.
- Dependencies and why the order matters.
- Data, compatibility, permission, concurrency, or rollback risk.
- Direct verification and completion signal.

Keep steps sized for review and progress tracking, not artificial five-minute
chunks. Separate must-have work from optional follow-up.

## Modes And Execution

- In Plan Mode, remain read-only and return the implementation plan.
- Outside Plan Mode, when the user asked for implementation and the next step is
  clear, update task progress and continue. Do not ask "shall I proceed?" merely
  because a plan exists.
- Create a plan document only when the user asked for one or the repository
  requires a durable design artifact.
- `/goal` defines the outcome and criteria; this skill designs the current route.
- Use `ha-workflow-script` only when a durable dynamic script is justified.

## Parallelism Decision

Mark steps parallel only when they are independent and have non-overlapping
writes or explicit worktree isolation. Use `ha-multi-agent-coding` for execution
strategy. A single broad investigation is not a batch fan-out.

## Quality Check

Before accepting the plan:

- Does it preserve user changes and existing contracts?
- Does every required criterion have an implementation and evidence path?
- Are risky state transitions and failure recovery covered?
- Is the plan small enough to execute but complete enough to close?
- Is the first actionable step clear?

## Smoke Prompts

- "Plan this cross-crate feature, then implement it."
- "Design a read-only migration plan with rollback and verification."
- "Turn these completion criteria into an executable coding plan."
