# 12 · Projects & Insights

This chapter covers how to **organize your work** (project containers, Custom Agents) and how to **understand your usage** (the Dashboard, Recap reports).

**In this chapter**

- [12.1 Project containers](#121-project-containers)
- [12.2 Custom Agents](#122-custom-agents)
- [12.3 The Dashboard](#123-the-dashboard)
- [12.4 Recap reports](#124-recap-reports)

---

## 12.1 Project containers

A project groups several related sessions into a single workspace that shares project instructions, project memory, and a unified working directory. Projects are **optional** — sessions that don't belong to a project are entirely unaffected.

**Creating one**: sidebar project tree → New project, then fill in the name, description, icon, color, default Agent, working directory, and project instructions.

**Key features**:

- **Working directory = a real folder**: a directory you explicitly choose, or the default `~/.hope-agent/projects/{project}/workspace/`. Uploaded files land directly in this directory, and the AI becomes aware of them through the working directory's top-level file listing and the `read` tool (there is no separate file table).
- **Project instructions**: the `AGENTS.md` at the root, which you can edit directly and which is assembled into the system prompt of every session under this project.
- **Project memory**: the highest priority (Project > Agent > Global); see [04 · Three tiers of memory](04-memory.md#41-three-tiers-of-memory-global--agent--project).
- **Lazy session creation**: opening "New chat" in a project doesn't pre-create an empty session — it stays a draft and is persisted only when you send the first message. Project sessions and Incognito are mutually exclusive.
- **Assigning an IM session to a project**: use `/project <id>` inside that IM chat to assign an existing session to the project directly (without creating a new session).

**The project settings panel** has four tabs: Overview (status + recent sessions), Files (a file browser where you can upload / delete / rename), Instructions (edit `AGENTS.md`), and Auto-memory (manage project auto-memory). You can also bind a Knowledge Space.

> **Deleting a project** proceeds in order: unbind sessions (the sessions themselves are kept) → delete the project → delete the project directory → delete the project memory. **An external directory you explicitly chose is never deleted.**

---

## 12.2 Custom Agents

Each Agent is a configuration on disk plus a few Markdown files that define who it is, which model it uses, and what capabilities it has. You can create multiple Agents (for example a "coding assistant" and a "writing assistant") that don't interfere with one another.

**Where to find it**: Settings → Agents. The editor page has eight tabs:

| Tab | Contents |
| --- | --- |
| Identity | Name, description, emoji, avatar, behavior notes |
| Personality | Role / vibe / tone / principles / boundaries, or free text |
| OpenClaw Compatible | Raw Markdown persona editing in an OpenClaw-compatible format |
| Tools & Skills | Tool / skill toggles, sandbox, approval, async policy, MCP toggles, and **the default permission mode for new sessions** |
| Model | Primary model / fallback chain / Plan-specific model / temperature / thinking effort (session > Agent > global) |
| Memory | This Agent's memory toggle and budget |
| Sub-Agent Invocation | See [09 · Sub-Agents](09-multi-agent-and-scheduling.md#91-sub-agents) |
| Approval | A custom list of tools that require approval |

**Which Agent a new session uses** is decided by a fixed priority order (the first non-empty one wins): explicit parameter → project default → IM topic / group / channel / account binding → **global default Agent** → built-in main Agent. So you can set a default Agent at three levels:

- **Global**: Settings → Agents (set in the "Default Agent" section).
- **Per project**: the default Agent in the project settings.
- **Per IM account**: the bound Agent in the channel account.

> The built-in main Agent cannot be disabled or deleted. When you delete an Agent, you must specify a replacement Agent (references are rebound to it), and the deletion is recoverable (moved to the recycle bin).

---

## 12.3 The Dashboard

The Dashboard aggregates and analyzes data across sessions, logs, and scheduled tasks, using charts to show cost, tokens, activity, health, and task execution.

**Where to find it**: the Dashboard in the sidebar, which has several tabs. The main dimensions include:

- **Overview**: session count, message count, input / output tokens, tool calls, errors, active Agents, active scheduled tasks, estimated cost, and average response time.
- **Token usage trend**: grouped by day / model / call type / purpose / domain.
- **Tool usage, session trends, error trends.**
- **Insights**: period-over-period comparisons, cost curves, an activity heatmap, top sessions, model cost-effectiveness, and a four-dimensional health score.
- **System metrics, local models, learning records.**
- **Goal & execution dashboard**: goal acceptance rate, workflow completion rate, strong-progress rate of ongoing advancement, and task and Plan breakdowns.
- **Capability Evaluation**: run core Agent synthetic scenarios with real models and inspect completion, tools, time, tokens, cost, comparisons, and trends. See [14 · Capability Evaluation](14-capability-evaluation.md) for the complete workflow.

**Model usage ledger**: every call that triggers model inference (chat, background queries, summarization, embedding, speech, judge, search, image generation, audio generation, vision, and so on) is recorded in a unified ledger, and the Dashboard's total tokens / cost are based on it. Calls that return no tokens — local models, speech, embedding — record only the call count and elapsed time; **they never fake accurate token counts with character-based estimates**.

> Session-level statistics automatically exclude scheduled tasks, Sub-Agents, and Incognito sessions; **Incognito sessions are never recorded in the ledger**. Cost estimation prefers the per-unit price of the model you configured (prices quoted in RMB are converted automatically); otherwise it falls back to the built-in pricing table.

---

## 12.4 Recap reports

Recap performs a deep analysis of the sessions in a selected time range and generates a multi-section review report that you can export as a standalone HTML file to share.

**Where to find it**: the Recap tab in the Dashboard, or the `/recap` command. Selectable ranges (incremental / 7 days / 30 days / 90 days).

**Report contents**: several AI analysis sections (project domains, interaction style, effective workflows, friction analysis, optimization suggestions, memory / skill recommendations, cost optimization, feature suggestions, future exploration, and more) plus the Dashboard's quantitative metrics and summary charts. **The exported HTML is a self-contained single file** (all styles inlined, zero external dependencies), supports both light and dark themes, and can be shared directly.

**Settings (Settings → Recap)**:

| Setting | Default | What it does |
| --- | --- | --- |
| Analysis model chain | Inherits global | Which models Recap analysis uses (decoupled from the main conversation, supports cross-model fallback) |
| Output language | Follows the interface | The report language |
| Default range | 30 days | The default range when there is no prior report |
| Max sessions per run | 500 | Upper limit |
| Cache retention days | 180 | How long report caches are kept |

---

## Next steps

- Find settings and understand the security boundaries → [13 · Settings & Security](13-settings-and-security.md)
- Let the AI advance goals in the background over the long term → [08 · Autonomous Tasks](08-autonomous-tasks.md)
- Validate Goal, Workflow, asynchronous-task, and multi-Agent stability with real models → [14 · Capability Evaluation](14-capability-evaluation.md)
