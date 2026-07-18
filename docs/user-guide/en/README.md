# Hope Agent User Guide

> [简体中文](../README.md) · **English**

> This is a complete user-facing manual that covers Hope Agent's installation, getting started, and the usage and settings for every feature.
> If you want to understand the code architecture and implementation details, see [`docs/architecture/`](../../architecture/); for a quick tour of the product highlights, see the root [README.md](../../../README.en.md).

---

## How to read this guide

- **First time using it**: read in order [01 · Getting Started](01-getting-started.md) → [02 · Models & Providers](02-models-and-providers.md) → [03 · Chat & Sessions](03-chat-and-sessions.md), and you'll be up and running in ten minutes.
- **Want a specific feature**: jump straight to the matching chapter—each chapter is self-contained and can be read on its own.
- **Want to change a setting**: start with the "Settings navigation map" in [13 · Settings & Security](13-settings-and-security.md); it tells you "which panel to go to for X".
- **Ran into an unfamiliar term**: see the [Core concepts](#core-concepts-all-in-one-place) section at the bottom of this page.

> Whenever the text mentions a **slash command** (such as `/goal`), you can type it directly in the chat input box; whenever it mentions a **settings panel**, it lives under "Settings" in the bottom-left corner of the app; whenever it mentions a **tool** (such as `exec` or `note_*`), that is a capability the AI invokes automatically during a conversation—you usually don't need to trigger it by hand, but you can govern it in the permission settings.

---

## What is Hope Agent

Hope Agent is a **local-first, desktop-first personal AI assistant that can also run as a resident service**. It opens and works like mature desktop software, yet has the real action-taking power of an Agent:

- **It remembers**—across sessions it remembers your preferences, project context, and long-term habits, understanding you better the more you use it;
- **It grows**—it distills complex tasks into reusable skills and turns your material into a second brain;
- **It advances goals autonomously**—you give it a goal, and it keeps breaking it down, executing, checking, and pushing it forward in the background;
- **It can truly operate**—under your authorization and approval, it runs commands, reads and writes files, controls the browser and your computer, and calls external tools;
- **It's always on call**—desktop, browser, and common IMs (Telegram / Feishu / Slack, etc.) share one set of sessions, memory, and configuration.

By default all data is stored on your own computer (`~/.hope-agent/`), and model requests connect directly to the Provider you configured, without passing through any third-party relay.

---

## Chapter navigation

| Chapter | Contents | When to read it |
| --- | --- | --- |
| [01 · Getting Started](01-getting-started.md) | Per-platform installation, the first-launch wizard, the three run modes, auto-update, and one-click local model install | You just got the app and want to get running fast |
| [02 · Models & Providers](02-models-and-providers.md) | Adding a Provider and API key, Codex sign-in, primary/fallback models, thinking effort and temperature, automatic failover, speech-to-text, media generation (images, speech, music, sound effects), web search | Configuring or switching models |
| [03 · Chat & Sessions](03-chat-and-sessions.md) | The chat interface, attachments and multimodal input, session management and search, incognito sessions, working directory, context compaction, and all slash commands | Everyday use of the chat features |
| [04 · Memory](04-memory.md) | The three memory tiers, auto-memory, on-demand recall, offline consolidation (Dreaming), user profile, and the correction loop | You want the AI to remember / forget certain things |
| [05 · Knowledge Space](05-knowledge-space.md) | Your second brain: creating a knowledge space, binding Obsidian, reading and writing notes, full-text and vector search, the backlink graph, and the AI chat panel | Managing notes and material |
| [06 · Design Space](06-design-space.md) | Generating 10+ kinds of artifacts—web pages, prototypes, presentations, and more—from a sentence, an image, or a URL, with live preview, fine-tuning, versions, export, and handoff to code | Doing design and deliverable artifacts |
| [07 · Tools & Permissions](07-tools-and-permissions.md) | The built-in toolbox, the three permission modes, the approval dialog, protected paths and dangerous commands, the Docker Sandbox, browser control, and computer control | Governing the AI's operating permissions |
| [08 · Autonomous Tasks](08-autonomous-tasks.md) | Goals to define results, workflows to orchestrate execution, Loop for continued scheduled progress, Plan Mode, task progress, and execution modes | Letting the AI push work forward in the background over the long term |
| [09 · Multi-Agent & Scheduled Tasks](09-multi-agent-and-scheduling.md) | Sub-Agents, Agent Teams, natural-language scheduled tasks, background jobs, and self-wakeup | Parallel collaboration and periodic tasks |
| [10 · IM Channels](10-im-channels.md) | Connecting Telegram / Discord / Slack / Feishu and more, multimodal, approvals, streaming mirror, and session handover | Using the AI inside chat tools |
| [11 · Connect & Extend](11-connect-and-extend.md) | The MCP client and platform server, Hooks lifecycle hooks, and the skill system | Connecting external tools and customizing behavior |
| [12 · Projects & Insights](12-projects-and-insights.md) | Project containers, Agent configuration, Dashboard cost and health, and Recap reports | Organizing work, reviewing, and managing cost |
| [13 · Settings & Security](13-settings-and-security.md) | The settings navigation map, changing settings by conversation (ha-settings), config backup and rollback, and security and reliability | Finding settings and understanding the security boundaries |

---

## Core concepts (all in one place)

Understanding the following groups of concepts will help you quickly make sense of the whole guide:

**Three run modes**—one core, three entry points:
- **Desktop GUI**: the most fully featured native app (full support on macOS, experimental on Windows / Linux), ready to use out of the box.
- **Server + Web GUI**: `hope-agent server` runs resident in the background (NAS / server / cloud host); open a browser to get the full web version, and phones and tablets can connect too.
- **ACP**: `hope-agent acp`, using Hope Agent as the AI backend for your IDE.

**The five roles of autonomous execution** (combine them or use each on its own):
- **Goal**—defines "what ultimately needs to be achieved and what counts as done".
- **Workflow**—handles "a single concrete execution", which can include stages, parallelism, multiple Agents, review, and verification.
- **Loop**—decides "when to push forward again" (on a schedule, a condition, or a time the model sets itself).
- **Task**—shows "the current progress".
- **Execution Mode**—controls "how aggressively it executes autonomously".

**Three dedicated spaces**:
- **Memory**—remembers important facts across sessions, split into Global / Project / Agent tiers and recalled on demand.
- **Knowledge Space**—a real Markdown note library that you and the AI read and write together, your "second brain".
- **Design Space**—a creative space for going from an idea to deliverable design artifacts.

**Tools and approval**—the AI actually operates your computer, files, browser, and external services through "tools"; sensitive operations enter an **approval** flow where you can confirm each time, always allow, or reject, and high-risk commands can even run inside a **Docker Sandbox**. Permissions have three modes: `Default`, `Smart Approval`, and `YOLO` (allow everything).

**Local-first**—by default, configuration, sessions, memory, attachments, skills, and logs all live in `~/.hope-agent/` on your machine; API keys connect directly to the model Provider; server mode provides Bearer Token authentication and SSRF protection.

---

## Quick task lookup

| I want to… | Where to go |
| --- | --- |
| Switch to a different model / add a new API key | [02 · Models & Providers](02-models-and-providers.md) |
| Use a local model without internet | [02 · Models & Providers · Local models](02-models-and-providers.md) |
| Have the AI remember my preferences / forget something | [04 · Memory](04-memory.md) |
| Manage my notes, bind Obsidian | [05 · Knowledge Space](05-knowledge-space.md) |
| Make a web page / poster / slide deck | [06 · Design Space](06-design-space.md) |
| Stop clicking "Allow" every time | [07 · Tools & Permissions · Permission modes](07-tools-and-permissions.md) |
| Have the AI keep working on something for me in the background | [08 · Autonomous Tasks](08-autonomous-tasks.md) |
| Do something on a daily schedule and send it to WeChat / Feishu | [09 · Multi-Agent & Scheduled Tasks · Scheduled tasks](09-multi-agent-and-scheduling.md#94-scheduled-tasks-cron) |
| Use it inside Telegram / Feishu | [10 · IM Channels](10-im-channels.md) |
| Connect external tools (MCP) | [11 · Connect & Extend](11-connect-and-extend.md) |
| See how much money / how many tokens I've spent | [12 · Projects & Insights · Dashboard](12-projects-and-insights.md) |
| Access Hope Agent on my computer from my phone | [01 · Getting Started · Run modes](01-getting-started.md) |
| Back up / restore my settings | [13 · Settings & Security](13-settings-and-security.md) |

---

_This guide is continuously updated alongside the product. If anything here does not match what you see in the app, the actual in-app display takes precedence, and we welcome your feedback on [GitHub Issues](https://github.com/shiwenwen/hope-agent/issues)._
