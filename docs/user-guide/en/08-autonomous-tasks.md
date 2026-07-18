# 08 · Autonomous Tasks

Hope Agent doesn't just answer questions—it can **keep working until the job is done**. This chapter introduces six roles that work together: **Goal** defines the outcome, **Workflow** organizes execution, **Loop** decides when to keep going, **Task** surfaces progress, **Execution Mode** controls how autonomous the agent is, and **Plan Mode** lets you plan before acting.

**In this chapter**

- [Mental model](#mental-model)
- [8.1 Goal](#81-goal)
- [8.2 Workflow](#82-workflow)
- [8.3 Loop](#83-loop)
- [8.4 Execution Mode](#84-execution-mode)
- [8.5 Plan Mode](#85-plan-mode)
- [8.6 Task](#86-task)

---

## Mental model

These roles can be combined or used on their own. Remember what each one is responsible for:

| Role | Question it answers | Command |
| --- | --- | --- |
| **Goal** | What are we ultimately trying to achieve? What counts as done? Which evidence counts? | `/goal` |
| **Workflow** | How exactly do we execute this one time? (phases, parallelism, review, verification) | `/workflow` |
| **Loop** | When do we push forward again? (schedule, condition, event) | `/loop` |
| **Execution Mode** | How intensely (how proactively, how deeply) should this session push forward? | `/mode` |
| **Plan Mode** | Write out the plan first and get my approval before acting | `/plan` |
| **Task** | Which step are we on right now? | (shown automatically) |

Their boundaries are clean: Goal only governs the endpoint and the evidence, and doesn't execute; Workflow only governs a single execution, and doesn't do long-term scheduling; Loop only governs triggering, and doesn't define completion criteria; Execution Mode only governs intensity.

---

## 8.1 Goal

Set a "final objective + completion criteria" for a long-running task, and the AI will keep pushing forward autonomously, giving you a short wrap-up and evidence when it's done.

**How to use it**:

```
/goal Ship the website v2 --criteria Home/Pricing/Docs pages all reachable with no console errors
/goal status      # View the current goal status
/goal pause       # Pause
/goal resume      # Resume
/goal evaluate    # Manually evaluate progress
/goal accept      # Accept the current result and close the goal
/goal clear       # Clear the goal
```

You can also enter through the "Goal" control in the input box. A summary of the current goal, its progress, and action buttons stays pinned above the input box; the workspace has a dedicated goal area where you can expand each completion criterion to see its status, budget, evidence, and timeline.

**A few key points**:

- A session can have only one goal in progress at a time.
- **Completion does not trust the AI's self-assessment**—it must pass a deterministic "final audit" (only complete when there is strong evidence, no blockers, and all required items are satisfied).
- Once complete, it waits for you to confirm and close: you can "accept and close," or "demand stricter evidence" to send the goal back to keep gathering evidence.
- After each round, if the goal still needs to advance, the system automatically schedules a follow-up run of about 10 seconds so the AI can continue, until it is complete / blocked / paused / out of budget.
- When creating a goal you can optionally set a **token / time / round budget** (leave blank = no limit); once reached, it refuses to start new workflows and warns you.

> [Incognito sessions](03-chat-and-sessions.md#36-incognito-sessions) cannot create goals; a session with a goal in progress also cannot be switched to incognito.

---

## 8.2 Workflow

With workflow mode on, when the AI faces a complex task it will **write its own execution script**, producing an execution process that is observable, pausable / resumable / cancelable, approvable, and recoverable after a crash.

**How to use it**:

```
/workflow on          # Turn on: allow the AI to orchestrate autonomously as needed
/workflow ultracode   # Stronger tier: for substantive tasks, default to considering multi-phase, parallel review, and cross-verification (slower and more expensive, quality first)
/workflow off         # Turn off
/workflow status      # Current mode + recent executions
/workflow trace <id>  # View the steps of a given execution
/workflow approve|pause|resume|cancel <id>
```

You can also toggle it from the workflow button in the input toolbar—clicking it opens a dropdown menu where you can directly choose Off / On / Ultracode (the current option is checked). Once on, just state your request as usual (for example, "research these three options and recommend one" or "do a full code migration"), and **the AI decides for itself whether creating a workflow is worthwhile**—simple tasks are still handled inline.

**What you'll notice**: phased execution, parallel exploration, multi-agent collaboration, built-in code review / verification / validation, and a bounded auto-fix loop; after a crash or restart it recovers conservatively (it won't silently mark itself complete, won't repeat completed steps, and won't auto-approve); you can choose where it runs (the current directory / a new isolated worktree / an existing worktree); and a completed workflow can be "saved as a template" for reuse.

> Turning on workflow mode is **not the same as running immediately**—it just gives the AI the ability to "orchestrate autonomously on the next round." Incognito sessions cannot use workflows. Workflow execution still goes through the normal [permission approvals](07-tools-and-permissions.md), sandbox, and incognito rules.

---

## 8.3 Loop

**Repeatedly trigger** the same task on a schedule, condition, or event. Each round can continue the current conversation, or trigger a workflow bound to a goal.

**How to use it**:

```
/loop 5m Check deployment status                    # Run once every 5 minutes
/loop Check deployment status                        # Let the AI decide the interval each round (self-paced)
/loop until CI is green every 5m: Fix the failing tests   # Conditional trigger
/loop status          # View all loops
/loop pause|resume|stop <id>
```

You can also create one from templates in the workspace's "Loop" center (check CI, refresh a report, summarize progress, and so on), choosing the trigger method and whether to "continue the conversation" or "execute via a workflow."

**Budgets and protections** (optional at creation): maximum number of triggers, maximum running window, token budget; plus **no-progress protection**—at the end of each round it deterministically judges whether real progress was made (it doesn't count "ran once" as progress); consecutive no-progress rounds first back off, and once the limit is reached it blocks and waits for your confirmation.

> Incognito sessions cannot create loops. A loop grants the AI no extra permission shortcuts—every round still goes through the session's existing permission mode and sandbox. For a self-paced loop, the interval the AI can choose is limited to between 1 minute and 1 hour.
>
> **Loop vs. self-wakeup**: a Loop is a repeated trigger; if the AI just wants to "call itself back after a while to continue once," it uses the one-shot [self-wakeup](09-multi-agent-and-scheduling.md#95-self-wakeup-schedule_wakeup).

---

## 8.4 Execution Mode

Set how intensely this session's AI applies "observe / plan / verify / fix" when pushing a task forward. It governs neither scheduling nor orchestration—only intensity.

**How to use it**: `/mode`, `/mode off|guarded|deep|autonomous`.

| Mode | Behavior |
| --- | --- |
| Off | Injects no advancement strategy |
| Guarded | Advances steadily; a failed verification records a fix event, and repeated failures / no progress can block |
| Deep | Same as Guarded, but allows deeper exploration and verification |
| Autonomous | The most autonomous; creating / running requires explicitly providing a runtime and output budget, otherwise it blocks |

---

## 8.5 Plan Mode

For tasks that aren't so simple, you can have the AI **write out the plan clearly and act only after you approve it**. The plan is the "design contract," the task list is the "source of truth for progress," and the two are independent.

**Entering is always your call**:

- You enter directly: the plan button in the input box, or `/plan enter`.
- The AI suggests entering: when it recognizes a complex task it pops up a Yes/No dialog; it enters only if you accept, and just gets to work if you decline.

**Five states**: Off → Planning (only the plan can change) → Review (the plan is locked) → Executing (the plan is frozen, and the task list tracks progress) → Completed. There is no "Paused" state—if it's suspended for a long time, exit (`/plan exit` is the escape hatch in any state) and re-enter when needed.

**The plan file** is a free-form Markdown design document; it's frozen after approval and unchanged during execution; the right-side plan panel renders it, with support for commenting on selected passages and version history. **The task list** is the sole source of truth for progress during execution.

> **Git checkpoint**: a Git checkpoint is created automatically when approval transitions to Executing, and cleaned up on completion or exit; you can roll back to this point at any time.

**Settings (Settings → Plan Mode)**:

| Setting | Default | Effect |
| --- | --- | --- |
| Plan sub-agent mode | Off | Use a separate sub-agent to draft the plan |
| Question wait timeout | Off (never times out) | Whether questions during planning time out, and after how many seconds |

---

## 8.6 Task

Tasks are the **user-visible progress** produced when the capabilities above execute, with three states: To do / In progress / Done. The AI automatically creates and updates the task list, shown above the input box; you can also mark items done manually.

The system also aggregates all these control planes into a session-level activity status—distinguishing "waiting on you" (needs approval / choice / completion confirmation) from "waiting on something external" (waiting on a subtask / file / timer), and only the former prompts you that you need to step in.

---

## Next steps

- Multi-agent collaboration and scheduled tasks → [09 · Multi-Agent & Scheduled Tasks](09-multi-agent-and-scheduling.md)
- Governing the permissions for this autonomous execution → [07 · Tools & Permissions](07-tools-and-permissions.md)
