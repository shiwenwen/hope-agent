---
name: ha-manual
description: "Answer 'how do I use X / where is setting Y / what does panel Z do' questions about Hope Agent from the built-in bilingual user guide instead of guessing from memory. Trigger on: 怎么用, 在哪设置, 怎么开启, 如何配置, 使用手册, 用户手册, 功能说明, how to use, where is the setting, how do I enable, user guide, manual. The guide covers install & onboarding, models & providers, chat & sessions, memory, knowledge space, design space, tools & permissions, autonomous tasks, multi-agent & cron, IM channels, MCP/hooks/skills, projects & insights, settings & security. NOT for: internal implementation questions (ha-self-diagnosis), error/log investigation (ha-logs), actually changing settings (ha-settings)."
whenToUse: "用户问某功能怎么用、在哪设置、如何开启，查内置使用手册作答。how to use a feature, where is a setting."
license: MIT
allowed-tools: [read, grep, find, ls, exec]
status: active
aliases:
  - manual
  - user-guide
---

# Hope Agent User Guide Lookup

You are answering a "how do I use this product" question. The full bilingual
user guide ships inside Hope Agent — read it and answer from it. Never invent
setting names, panel paths, or slash commands from memory: if the guide names
a panel, quote that name; if you cannot find the answer in the guide, say so.

## Resolve the manual root first

The guide is mirrored to `<data-dir>/manual/` where `<data-dir>` is
`$HA_DATA_DIR` when set (Docker: `/data`), else `~/.hope-agent`. Resolve it
dynamically — do NOT hard-code `~/.hope-agent` (wrong under Docker/portable):

```bash
ls "${HA_DATA_DIR:-$HOME/.hope-agent}/manual/"
```

Layout: `manual/zh/` (简体中文) and `manual/en/` (English) — pick the language
the user is speaking. Each contains `index.md` (overview + navigation) and
`01.md` … `13.md` (chapters). If the directory is missing (mirror not ready:
first boot still warming, or disk issue), tell the user the manual is still
being prepared and to retry in a moment — do not fall back to other paths;
this mirror is the only on-disk copy in packaged installs.

## Iron rules

- **Read-only**: only `read` / `grep` / `find` / `ls` the manual tree.
- **Answer from the text**: cite the chapter number + section heading you
  used (e.g. "见 05 · 知识空间 §5.2"). Quote exact UI labels from the guide.
- **Don't dump chapters**: extract the relevant section, answer concisely,
  and point to the chapter for more.

## Chapter routing table

Pick the chapter from the question, `read` it, `grep` only when unsure:

| File | Chapter | Covers |
| --- | --- | --- |
| `01.md` | 快速上手 / Getting Started | install per platform, first-launch wizard, run modes (desktop / server+web / ACP), updates, one-click local models, remote access |
| `02.md` | 模型与 Provider / Models & Providers | providers & API keys, Codex sign-in, primary/fallback models, thinking & temperature, failover, STT, media generation (image/audio), web search |
| `03.md` | 对话与会话 / Chat & Sessions | chat UI, attachments & multimodal, session management & search, incognito, working directory, context compaction, all slash commands |
| `04.md` | 记忆系统 / Memory | three memory tiers, auto-memory, recall, Dreaming, user profile, correction loop |
| `05.md` | 知识空间 / Knowledge Space | notes, binding Obsidian, full-text & vector search, backlinks & graph, AI chat panel, access control |
| `06.md` | 设计空间 / Design Space | generating web pages / posters / decks etc., live preview, fine-tuning, versions, export, handoff to code |
| `07.md` | 工具与权限 / Tools & Permissions | built-in tools, permission modes, approval dialog, protected paths & dangerous commands, Docker sandbox, browser control, computer control |
| `08.md` | 自主任务 / Autonomous Tasks | goals, workflows, Loop, Plan Mode, task progress, execution modes |
| `09.md` | 多 Agent 与定时任务 / Multi-Agent & Scheduling | sub-agents, agent teams, natural-language cron jobs, background jobs, self-wakeup |
| `10.md` | IM 渠道 / IM Channels | Telegram / Discord / Slack / Feishu etc., approvals over IM, streaming mirror, session handover |
| `11.md` | 连接与扩展 / Connect & Extend | MCP client & platform server, hooks, skill system |
| `12.md` | 项目与数据洞察 / Projects & Insights | project containers, agent configuration, dashboard cost & health, Recap reports |
| `13.md` | 设置与安全 / Settings & Security | settings navigation map ("which panel changes X"), changing settings via chat, config backup & rollback, security & reliability |
| `14.md` | 能力评测 / Capability Evaluation | evaluation center: repeatable synthetic tasks against real models, task success / tool calls / latency / token cost metrics, orchestration stability |
| `index.md` | 目录 / Overview | reading guide, core concepts glossary, "I want to…" quick lookup |

Not sure which chapter? `13.md` has the settings navigation map, and
`index.md` has an intent → chapter lookup table.

## When NOT to use this skill

- How Hope Agent works **internally** (source/architecture) → `ha-self-diagnosis`
- Something is **broken** / error investigation → `ha-logs`
- Actually **changing** a setting for the user → `ha-settings`

$ARGUMENTS
