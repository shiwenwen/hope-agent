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

Have the AI automatically run a conversation on a schedule; the result can raise a desktop notification or be delivered to designated IM chats.

### Ways to create

- **Natural language**: just tell the AI "remind me to drink water at 9 a.m. every day" and it will create the task for you.
- **Form**: Scheduled Tasks panel → New, then fill in the name, schedule, message, Agent, delivery targets, and so on.

### Schedule types and time zones

| Type | Description |
| --- | --- |
| One-off (At) | Fires once at a specified date and time |
| Fixed interval (Every) | Fires every N minutes / hours / days (minimum 1 minute) |
| cron expression (Cron) | Standard cron, with a visual builder in the GUI; includes an **IANA time-zone picker** that defaults to your browser's time zone, interprets the schedule against that zone's wall clock, and is daylight-saving aware |

### Delivering results to IM

The final text of a task's result can be sent to one or more IM chats—first pick the channel account, then the chat.

- **Delivery allowlist**: a target must be a real, recorded IM chat, which prevents an injected AI from turning a recurring task into an exfiltration channel.
- **Prefix toggle** (off by default): on a successful delivery, prepend a `[Cron] <task name>` prefix so multiple tasks delivered to the same group are easy to tell apart.
- When a deleted account makes a target invalid, the UI flags it in red and delivery skips it.

### Per-job settings (owner only)

| Setting | Default | Description |
| --- | --- | --- |
| Permission-mode override | Follow the Agent | Use Default / Smart Approval / YOLO for this task; **can only be set in the UI—AI tools cannot change it** (to prevent privilege escalation) |
| Sandbox-mode override | Follow the Agent | Choosing anything other than "Off" requires Docker; if it is unavailable the run is aborted and never runs unsandboxed |
| Timeout override | Use the global value | Lets a long task declare its own budget without raising the global value |
| Maximum failures | 5 | Auto-disable after this many consecutive failures (0 = never auto-disable) |

When a task reaches the consecutive-failure limit it is **auto-disabled and raises a dedicated notification**; infrastructure-type failures (the session never started) don't count toward it.

### Centralized view and unread

Every run creates a fresh isolated session (its title is the task name) and **no longer mixes into the main sidebar session list**. You can review them all in one place in the "History" view of the Scheduled Tasks panel: the left column is a cross-task run timeline, and the right column is the full conversation for that run (read-only). The Scheduled Tasks icon in the sidebar has its own unread badge with a one-click "Mark all as read." Scheduled-task unread is independent of ordinary conversations and does not count toward the Dock / tray.

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
| Where it returns | The current session | A new isolated session each time |
| Best for | Short, temporary waits (waiting for some task to finish) | Long-period / recurring / delivery tasks |

The delay ranges from 10 seconds to 24 hours (the default cap, adjustable up to 7 days in settings), with at most 5 pending wakeups per session.

---

## Next steps

- Use it in Telegram / Feishu → [10 · IM Channels](10-im-channels.md)
- Connect external tools → [11 · Connect & Extend](11-connect-and-extend.md)
