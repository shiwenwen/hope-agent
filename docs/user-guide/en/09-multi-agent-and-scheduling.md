# 09 · Multi-Agent & Scheduled Tasks

This chapter covers the capabilities that let work run in parallel and on a schedule: handing subtasks to **Sub-Agents**, having several members of an **Agent Team** collaborate, running work periodically with **Scheduled Tasks** and delivering the results to IM, and the AI's **self-wakeup**.

**In this chapter**

- [9.1 Sub-Agents](#91-sub-agents)
- [9.2 Agent Teams](#92-agent-teams)
- [9.3 The background jobs panel](#93-the-background-jobs-panel)
- [9.4 Scheduled tasks (Cron)](#94-scheduled-tasks-cron)
- [9.5 Self-wakeup (schedule_wakeup)](#95-self-wakeup-schedule_wakeup)

---

## 9.1 Sub-Agents

The main Agent can hand an independent subtask to another Agent (or the same one) to run asynchronously in an **isolated session**; when it finishes, the result flows back into the current conversation automatically so the main Agent can carry on. The AI drives all of this on its own—you don't do anything by hand—but you can govern it in settings.

- **Foreground wait**: it waits 30 seconds by default; if the subtask hasn't finished by then, it automatically moves to the background to keep running without blocking your conversation.
- **Batch dispatch**: dispatch several subtasks in parallel at once; when they all finish, they are merged into a single summary and injected in one shot (saving a round of cost).
- While they run, you can steer them (push hints) and cancel one or all of them.
- By default, Sub-Agents run in their own [isolated worktree](07-tools-and-permissions.md#79-file-operations-git-and-isolated-worktrees), so they don't pollute your main workspace.

**Watching them run**: each Sub-Agent appears in the conversation as a capsule (avatar, name, status, elapsed time); click one to open the "Sub-agents" panel on the right.

- The list is grouped into **running / finished** and covers every Sub-Agent in this session—foreground, background, and the ones a Workflow dispatched. Continuing the same Sub-Agent keeps it as one item and shows its accumulated run count.
- Open one to see its **result**, its **invocation details** (provider, model, thinking level, start/finish time, attachments, nesting depth, and more), its **live conversation**, and the timeline of all its runs; finished runs land on the result, running ones land on the conversation.
- If the app exits or its process is interrupted, any Sub-Agent that was running is shown as **Interrupted** instead of being silently restarted. The main Agent or a Workflow can explicitly continue that same Sub-Agent: its existing conversation and working directory are preserved, while a new run is created. A run the user cancelled is not resumed automatically.
- If a Sub-Agent dispatched its own, you can keep drilling in and walk back up via the breadcrumb.
- The workspace panel carries the same roster in a "Sub-agents" section.

**Settings** (in the Agent editor → Sub-Agent Invocation tab):

| Setting | Default | What it does |
| --- | --- | --- |
| Enable Sub-Agents | On | Whether this Agent may dispatch Sub-Agents |
| Allowed Sub-Agents | All | Which Agents may be used as Sub-Agents |
| Maximum nesting depth | 3 | How many levels a Sub-Agent may itself dispatch Sub-Agents (1–5) |
| Maximum concurrency | 8 | Cap on Sub-Agents running at once in a single session |
| Default timeout | 0 (no timeout) | Sub-Agent execution timeout, in seconds |

> Messages you send take priority over automatic injection—sending a message cancels an in-flight injection, and the injection is queued and retried once you're idle (no result is lost). Background Sub-Agents are projected into the unified [background jobs panel](#93-the-background-jobs-panel).

---

## 9.2 Agent Teams

A team lets several named Agents work as members that **collaborate in parallel**—members can message one another and share a single kanban board of tasks, all orchestrated by a coordinator. This differs from the one-way "parent dispatches, child returns" pattern of Sub-Agents.

**How to use it**:

- The AI creates one by calling the `team` tool (a team template lays out the members in one click), or you manage teams with the `/team` command.
- **Templates are pre-configured in settings**: Settings → Teams, add / edit templates; each template defines the members' names, the Agent each is bound to, and their roles.
- While a team runs, a team panel opens on the right with three tabs: Dashboard (member cards + progress + token stats), Tasks (four columns: To do / In progress / Review / Done), and Messages (a live message stream where you can send messages to the team by hand).

**Settings** (in the Agent configuration): whether creating teams is allowed, the maximum number of active teams (default 3), the maximum members per team (default 8), and the members' default model.

> There are **no built-in team templates** yet—you define them all yourself in settings. If an Agent referenced by a template is deleted, creating the team will raise an error.

---

## 9.3 The background jobs panel

Every asynchronous / background tool, Sub-Agent, and batch job lands in a single panel where you can watch status and running output in real time and cancel at any time.

- **Entry points**: the badge in the chat header, the standalone panel, and the background-jobs section of the workspace.
- When a command runs in the background you can watch the output "tail" live to tell whether it is "still running" or "stuck."
- When a job finishes it can raise a desktop notification (on by default).
- In the chat message stream, control actions like pause / resume / cancel (for Sub-Agents, background jobs, processes, teams, workflows, and scheduled tasks) are grouped into a single readable status line instead of raw tool calls; the group is collapsed by default and auto-expands when there is a failure or partial failure; a cancel is shown accurately as "cancelled" rather than being misreported as "completed."

Which tools can run in the background: command execution, browser, web search, AI image generation, and app update.

**Settings (Settings → Tool Settings → Async Tools, medium risk)**:

| Setting | Default | What it does |
| --- | --- | --- |
| Enable async tools | On | Master switch for backgrounding |
| Auto-background threshold | 0 (off) | A synchronous tool that runs longer than this many seconds is moved to the background automatically |
| Completion merge window | 3 seconds | Multiple jobs in the same session that finish within this window are merged into a single injection round (to save cost) |
| Global concurrency cap | 8 (based on core count) | Number of background jobs running at once across all sessions |
| Per-session concurrency cap | 6 (derived from core count, always less than the global cap) | Number of background jobs running at once in a single session; anything beyond queues |
| Automatic retry on failure | Off | Retries only tools with no side effects (such as web search); command execution, image generation, and the like are never retried |

---

## 9.4 Scheduled tasks (Cron)

Have the AI automatically run a conversation on a schedule. Each occurrence can create a new ordinary chat or join an existing chat's queue; the result can also raise a desktop notification or be delivered to designated IM chats.

### Ways to create

- **Create with Model**: in Scheduled Tasks, click the main New Task button or choose **Create with Model** from its arrow menu. The app returns to an ordinary chat and prefills a conversational starter; finish the time and action, then send it to the model. The model can create either a new-chat task or, when you explicitly ask, arrange the current chat.
- **Set Up Manually**: choose **Set Up Manually** from the New Task arrow menu, then fill in the name, schedule, message, Agent, workspace, delivery targets, and so on. By default, each occurrence creates a new ordinary chat; switch **Run in** to an existing chat to search for and pick any ordinary chat as the target.
- **Schedule the current chat**: click the timer icon, "Schedule this chat," in an ordinary chat's title bar. The target is locked to that chat, so you do not have to select its Agent or project again.

You can also tell the model, "In one minute, remind me in this chat to continue testing," or "Every day at 9 AM, summarize progress in the chat named 'Website redesign'." The first binds the current chat directly. For another chat, the model first discovers existing sessions and then uses the exact requested chat id rather than guessing from a title. A generic "remind me" still defaults to a separate new-chat task.

### Choose where it runs

| Destination | How it runs | Best for |
| --- | --- | --- |
| New chat | Each occurrence creates an ordinary chat titled after the task | Standalone reports, checks, and tasks where every full run should remain separate |
| Existing chat | Every occurrence enters the same chat as a read-only Scheduled message; **Schedule this chat** is the shortcut for the current one, and manual setup can search for another chat | Work that should continuously share the same history, project, knowledge spaces, Worktree, or working directory |

The destination is fixed once the task exists; create a new task to run in a different chat.

New run chats appear normally in the main sidebar, search, pins, archives, and ordinary unread. They are not hidden Cron-only sessions. A scheduled-trigger badge connects the message to its source task.

For an existing-chat task, the system reads the chat's **live** Agent, model, project, knowledge spaces, working directory, permission mode, and sandbox mode when the occurrence actually reaches the front of the queue. The task neither copies nor temporarily overrides these settings. If you change the chat later, the next occurrence follows the new configuration.

### Schedule types and time zones

| Type | Description |
| --- | --- |
| One-off (At) | Fires once at a specified date and time |
| Fixed interval (Every) | Fires every N minutes / hours / days (minimum 1 minute) |
| cron expression (Cron) | Standard cron, with a visual builder in the GUI; includes an **IANA time-zone picker** that defaults to your browser's time zone, interprets the schedule against that zone's wall clock, and is daylight-saving aware |

### Checks before saving or running now

A preflight appears before you create, edit, or choose "Run now." It leads with the conversation this task will run in — a new chat, or the specific existing chat and its current title — then shows the next three run times and the actual Agent, primary model, project, workspace mode / base ref, number of dirty workspace files, permissions, and sandbox. Delivery targets are listed one by one with their individual problems, and the footer shows whether this process is the scheduler's primary, how many tasks are running against the scheduler-wide concurrency cap, and whether this task itself is running right now.

- A **blocker** cannot be forced through. Common causes include an unavailable target chat, an archived or missing managed project, an invalid base ref, a busy or conflicted Persistent Worktree, an unconfigured model, an invalid IM account / chat, or disabled remote writes.
- A **warning** can be confirmed after you understand its impact.
- If another window or process just changed the task, the form reports a revision conflict and preserves your draft. Fields are replaced only when you explicitly load the latest version; a stale draft never silently overwrites newer settings.

### Workspace modes

Only new-chat tasks have a task-level workspace choice. Current-chat tasks always follow the workspace already used by their target chat.

| Mode | Behavior | After a run |
| --- | --- | --- |
| Project | Runs directly in the project's working directory | Simplest; it also sees existing changes in that directory |
| Fresh | Creates an isolated Git Worktree from an optional base ref for every occurrence | Each result is retained separately; archive, restore, or discard it when those actions are offered |
| Persistent | Keeps one task-owned Git Worktree and reuses it across occurrences | Every run chat keeps that Worktree mounted and can be reopened like an ordinary chat; take it over to pause the task for exclusive manual work |

Fresh creates a full new Worktree per run and keeps it until you archive or discard it. When such a task usually changes nothing — checks, read-only analysis — set **After each run** to "Remove if unchanged": the Worktree is deleted only when that run left no changes and no new commits, so nothing you might still need is removed. "Always remove" deletes it including uncommitted changes, which suits only tasks whose real output is delivered elsewhere. A Worktree you have taken over, or one still in use, is never cleaned up automatically.

Fresh and Persistent require a Git project. A blank base ref means `HEAD`; preflight verifies that it resolves to a commit. The backend decides which actions are safe from the Worktree's real ownership, run state, conflicts, and chat custody, and fails closed if that state cannot be read. Unresolved Persistent or retained Worktrees remain discoverable in Scheduled Tasks even after their task is deleted.

Finishing a run does not unmount its Worktree from the conversation. Reopening that chat from the message list or run history keeps the same Worktree path in the title bar, file panel, and tool runtime, just like an ordinary Worktree-backed chat.

Natural-language creation supports all three modes too. For example, ask: "In project X, create a Fresh Worktree from `main` and check dependencies every day," or "Give project X a Persistent Worktree and continue fixing tests every hour." The AI resolves the Project, stores the workspace mode and base ref, and runs the same preflight as the form. An invalid ref, a non-Git Project, or a busy / conflicted Persistent resource is a blocker. You can also ask the AI to inspect a scheduled task's workspace status or change its mode and base ref while no run or retained Persistent resource owns that policy.

After a successful AI-created task, the reply includes a Scheduled Task card. It shows the live status, schedule, next run, and Worktree mode; choose **Open** to jump directly to task details. The card is persisted with chat history and remains usable after reopening the chat.

Conversation control covers the task's Project, workspace mode, and base ref. Permission-mode and sandbox overrides remain user-only UI settings: the AI cannot relax permissions, lower the sandbox, or modify a task that already carries owner-configured permission overrides.

### Chat experience while a run is active

Task details do not embed a message list. Select any **run history** row to enter its ordinary chat directly and focus that occurrence, where the full composer, live streaming output, and follow-up conversation are available.

A Scheduled message in an existing chat has a timer badge and is managed by the system: it cannot be edited, deleted, or force-inserted. It shares one durable FIFO with ordinary messages from the desktop, web, and IM. Earlier messages run first; a later Scheduled occurrence shows as queued / waiting and never jumps ahead of an active turn or user-owned queued content. Ordinary messages may still offer "send after reply" or explicit insertion on supported surfaces, subject to the current dispatchable-head rules.

### History, badges, and unread

- Scheduled Tasks provides task-list, calendar, and cross-task run-history views. History addresses a **specific occurrence** and shows its status, duration, result, delivery, and Worktree information.
- You can also just ask in chat: "Why did the daily report task fail last time?" The AI reads that task's recent runs — status, duration, error, result summary — to answer. To stop the occurrence executing right now, say "cancel the scheduled task that is running"; that ends only this run and leaves the schedule intact. Say "pause" to stop future runs instead.
- List search matches the task name, its description, and the last run's error or result summary, and the status filter has a **Needs attention** entry that keeps only the tasks still waiting on you. A troubled task states its reason on the row itself — auto-disabled after consecutive failures, last run failed, missed occurrence, stale delivery target — with the matching entry point: resume an auto-disabled task in one click, jump straight to editing a stale delivery target, or open the run history for the rest.
- A new-chat occurrence opens its ordinary chat. An existing-chat occurrence opens the exact target message for that run, rather than pretending the latest turn in that shared chat is the requested history.
- The timer card on a message shows the task name, can expand the original creation prompt, and links back to the task; the title bar also shows the source task. If the task was deleted, a "Task deleted" tombstone preserves the history.
- When the model creates a task, the chat keeps a task card: while the task exists it shows the status and next run and opens the task details; if the last run failed, **Run Again** starts another occurrence right from the card; once the task is deleted the card is marked as deleted and offers **Copy as New Task**, which opens a new-task draft with the original configuration.
- The Scheduled badge is a projection of the ordinary chat read state, not a second unread system. Rendering the Scheduled preview or ordinary chat advances the same watermark. Archived chats do not count, multiple runs in one chat count as one unread chat, and Dock / tray unread follows the same ordinary-chat rules.

### Stop, cancel, and delete

- **Queued**: canceling from run history removes that exact Scheduled message without affecting other occurrences or ordinary messages.
- **Running**: cancel from history or choose Stop in the ordinary chat to stop only this occurrence. It does not pause future schedules and does not require Continue before the next occurrence.
- **Global Stop**: a global stop durably holds Scheduled messages that have not begun. They return to the queue only after the corresponding exact Continue, and an app restart cannot bypass the fence.
- **Delete task**: deletion is soft. The task no longer triggers or delivers, but run logs and already-created ordinary / legacy sessions remain. An active occurrence is precisely canceled first, or deletion fails closed. After deletion you can still use **Copy as New Task** on the chat's task card to rebuild it from the original configuration.

Compatibility note: legacy `SessionLoop` runs do not have a durable `turnId`, so they do not show Stop while running; deleting their task is also safely refused until they finish. New ordinary-chat and existing-chat runs both support exact cancellation.

### Delivering results to IM

The final text of a task's result can be sent to one or more IM chats—first pick the channel account, then the chat.

- **Delivery allowlist**: a target must be a real, recorded IM chat, which prevents an injected AI from turning a recurring task into an exfiltration channel.
- **Prefix toggle** (off by default): on a successful delivery, prepend a `[Cron] <task name>` prefix so multiple tasks delivered to the same group are easy to tell apart.
- Creation and editing validate the account, channel, and chat. A disabled, deleted, stale, or otherwise invalid target blocks preflight or is safely skipped at delivery time; the system never guesses where to send.

### Per-task settings (owner only)

| Setting | Default | Description |
| --- | --- | --- |
| Agent / project | Auto / none | Available only for new-chat tasks; an existing-chat task follows the target chat's live configuration |
| Permission-mode override | Follow the Agent | Use Default / Smart Approval / YOLO for this task; **can only be set in the UI—AI tools cannot change it** (to prevent privilege escalation) |
| Sandbox-mode override | Follow the Agent | Choosing anything other than "Off" requires Docker; if it is unavailable the run is aborted and never runs unsandboxed |
| Timeout override | Use the global value | Lets a long task declare its own budget without raising the global value |
| Maximum failures | 5 | Auto-disable after this many consecutive failures (0 = never auto-disable) |

When a task reaches the consecutive-failure limit it is **auto-disabled and raises a dedicated notification**; infrastructure-type failures (the session never started) don't count toward it.

### Common states and what to do

| State / symptom | Meaning and action |
| --- | --- |
| Preparing / Queued | The occurrence is durably recorded and preparing or waiting for earlier messages / a global concurrency slot; do not click Run now repeatedly |
| Running | The model or tools are executing; use Stop / cancel when exact cancellation is available |
| Completing | The model has finished and the system is settling its Worktree, ledger, or delivery; Stop is no longer accepted |
| Cancelled | Only this occurrence was canceled; the future schedule remains active |
| Paused / Needs attention | The target chat, project, Worktree, or delivery setup needs action; fix it and explicitly resume the task |
| Task deleted | The task no longer triggers, but its history and retained resources remain available for review / cleanup |

An invalid or archived existing-chat target pauses a recurring task instead of generating the same error every period. Infrastructure failures do not count toward the consecutive-failure limit. A one-off task that hits infrastructure trouble before model execution follows the safe retry rules, while cancellation is not misclassified as failure.

### Global settings (Settings → Scheduled Tasks, medium risk)

| Setting | Default | What it does |
| --- | --- | --- |
| Maximum concurrency | 5 | Scheduler-wide concurrency cap (0 = unlimited) |
| Global timeout | 0 | Time budget per run (0 = no timeout) |
| Catch-up window | 300 seconds | The window to catch up a missed one-off task (0 = strictly no catch-up) |

---

## 9.5 Self-wakeup (schedule_wakeup)

This is the AI's one-off "call me back into the current session in N seconds to continue" capability, which differs from Scheduled Tasks:

| Dimension | Self-wakeup | Scheduled Tasks |
| --- | --- | --- |
| Semantics | The AI proactively says "call me back in a bit" | A standalone planned task |
| Count | **One-off** | Once or recurring |
| Where it returns | The current session | A new ordinary chat, or an existing chat you choose |
| Best for | Short, temporary waits (waiting for some task to finish) | Long-period / recurring / delivery tasks |

The delay ranges from 10 seconds to 24 hours (the default cap, adjustable up to 7 days in settings), with at most 5 pending wakeups per session.

---

## Next steps

- Use it in Telegram / Feishu → [10 · IM Channels](10-im-channels.md)
- Connect external tools → [11 · Connect & Extend](11-connect-and-extend.md)
