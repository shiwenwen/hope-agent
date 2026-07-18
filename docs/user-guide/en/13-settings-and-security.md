# 13 · Settings & Security

This chapter is the "map" to the settings plus a security overview: it tells you **which panel to go to when you want to change something**, which settings the AI can change for you through conversation, how to back up and roll back your configuration, and what Hope Agent does for data security and reliability.

**In this chapter**

- [13.1 Settings navigation map](#131-settings-navigation-map)
- [13.2 Changing settings by conversation](#132-changing-settings-by-conversation)
- [13.3 Config backup and one-click rollback](#133-config-backup-and-one-click-rollback)
- [13.4 Data and security](#134-data-and-security)
- [13.5 Reliability: keep-alive and crash recovery](#135-reliability-keep-alive-and-crash-recovery)
- [13.6 Reporting problems and self-diagnosis](#136-reporting-problems-and-self-diagnosis)
- [13.7 Other common settings](#137-other-common-settings)

---

## 13.1 Settings navigation map

The left side of the Settings page is a navigation column. The table below tells you "to change X, go to which panel":

| Panel | What it controls |
| --- | --- |
| **Profile** | Avatar, name, time zone, preferred reply language, AI experience level, reply style |
| **General** | Appearance (theme / interface language / sidebar and chat display mode / background animation), System (launch at startup / prevent sleep / global shortcuts / rerun onboarding), Network (proxy) |
| **Model Configuration** | [Providers, API keys, primary / fallback models, media generation models](02-models-and-providers.md) |
| **Agents** | [Creating / editing Agents](12-projects-and-insights.md#122-custom-agents) |
| **Teams** | [Agent Team templates](09-multi-agent-and-scheduling.md#92-agent-teams) |
| **IM Channels** | [IM Channel accounts](10-im-channels.md) |
| **Skills** | [Skill enablement / draft review / installation](11-connect-and-extend.md#114-the-skill-system) |
| **Tool Settings** | General, Web search, Web fetch, Media generation, Canvas, Async tools, Issue reporting (the weather widget is configured inside the "General" sub-page) |
| **MCP Servers** | [MCP connections](11-connect-and-extend.md#111-mcp-connecting-external-tools) |
| **Memory** | [Memory extraction / recall / budget, embedding, Dreaming](04-memory.md) |
| **Knowledge** | [Knowledge bases, retrieval, passive recall, autonomous maintenance, Sprite](05-knowledge-space.md) |
| **Design Space** | [Design Space toggles, auto-preview, export parameters](06-design-space.md#610-settings) |
| **Chat & Context** | Basic chat behavior, behavior awareness, context compaction |
| **Scheduled Tasks** | [Concurrency limit, timeout, catch-up window](09-multi-agent-and-scheduling.md#94-scheduled-tasks-cron) |
| **Speech-to-Text** | [Speech-to-Text (STT)](02-models-and-providers.md#211-speech-to-text-stt) |
| **Plan Mode** | The sub-agent and question timeout for [Plan Mode](08-autonomous-tasks.md#85-plan-mode) |
| **Recap** | The model, language, and scope for [Recap reports](12-projects-and-insights.md#124-recap-reports) |
| **Server** | [Run mode, listen address, API key, remote access](01-getting-started.md#14-access-from-your-phone-or-another-computer) |
| **Files** | File size limits (attachment / upload / preview / edit / document) |
| **Docker Sandbox** | [Docker Sandbox](07-tools-and-permissions.md#76-docker-sandbox) container configuration |
| **Browser** | [Browser backend, extension, raw CDP toggle](07-tools-and-permissions.md#77-browser-control) |
| **ACP Control Plane** | Delegation to external ACP Agents |
| **Notifications** | Master notification switch, reply preview, background-task alerts, IM online alerts |
| **Permission** | [Approval policy, timeout, protected paths / dangerous commands, smart mode, unattended](07-tools-and-permissions.md) |
| **Hooks** | [Lifecycle hooks](11-connect-and-extend.md#113-hooks-lifecycle-hooks) |
| **Permissions** | [macOS system permissions (Accessibility / Screen Recording, etc.)](01-getting-started.md) |
| **Security Policy** | Dangerous mode (skip all approvals globally), SSRF outbound policy |
| **System Health** | Guardian toggle, crash records, full backup and recovery |
| **Logs** | Log viewer |
| **About / Update history** | [Version, check for updates, auto-update](01-getting-started.md#16-updating-to-a-new-version) |

> The system tray's behavior (it only shows whether there is a red dot) is fixed and has no separate setting.

---

## 13.2 Changing settings by conversation

Besides clicking in the interface, you can also **ask the AI to change settings for you directly in conversation**—for example, "switch the theme to dark," "set the tool timeout to 60 seconds," "turn off notifications." The AI reads and writes settings through a built-in capability and **never edits the config file directly**.

It confirms with you in three risk tiers:

- **Low risk** (appearance / display preferences): changed with a one-line note.
- **Medium risk** (things that affect context / cost / output quality, such as compaction, memory, search, approvals): it shows you "current value → new value" before changing.
- **High risk** (security / network exposure / credentials / permission rules / requires restart, such as proxy, global shortcuts, dangerous mode, server, auto-update, protected paths, etc.): it **only changes them after you explicitly confirm**.

**A few classes of settings the AI can only read, not change—you can only change them in the interface** (for credential safety and runtime stability):

- **The Provider list and API keys**
- **The media generation provider list and API keys** (the default model chains and generation parameters can still be changed by conversation)
- **IM Channel accounts**
- **MCP server configuration**
- **The choice of primary / fallback models**
- **The choice of embedding (vector) model**
- **The Speech-to-Text (STT) provider**
- **Hooks configuration**

Even for settings the AI can read, credential fields (API keys and the like) are automatically **masked** on read, so they never appear in the conversation.

---

## 13.3 Config backup and one-click rollback

Before every config change, the system automatically keeps a copy of the old configuration, so you can roll back at any time if something breaks—and **the rollback action itself can be undone as well**.

- **Automatic snapshots**: automatically archived before every config write, keeping the most recent 50; the filename records "who changed what and when."
- **Full crash backup**: when crash diagnosis is triggered, or when you trigger it manually, the entire config directory is bundled up, keeping the most recent 5.
- **Restore defaults**: every settings panel has a restore control in its title bar that restores the defaults for that page / that section.
- You can also ask the AI in conversation to list backups and roll back to a snapshot (this is high risk, so it shows you the timestamp and contents for confirmation first).

After a config change, the interface hot-reloads automatically (theme, language, notifications, etc.), and most changes require no restart.

---

## 13.4 Data and security

**Local-first**—all data (sessions, memory, skills, config, credentials) is stored in the `~/.hope-agent/` directory on your own machine and never uploaded to the cloud. API keys are used only to **call the Providers you configured directly**; Hope Agent does no relaying of any kind.

| Topic | Details |
| --- | --- |
| **You own your data** | All data lives in `~/.hope-agent/`; [Incognito sessions](03-chat-and-sessions.md#36-incognito-sessions) are "burn on close" and never enter the sidebar / search / statistics |
| **Direct API-key connection** | Keys are used only to connect directly to the model services you configured, never through a third party |
| **Logs never contain keys** | All logs are forcibly redacted; API keys / OAuth tokens are forbidden from appearing in any log |
| **Server-mode authentication** | The HTTP/WS service authenticates with a Bearer Token (`/api/health` is exempt); **be sure to set an API key** before binding `0.0.0.0` and exposing it externally |
| **SSRF protection** | All outbound requests undergo mandatory safety checks and by default block access to intranet and cloud-metadata addresses; you can adjust the policy and add a trusted-host allowlist in Settings → Security Policy |
| **Credential storage** | Credentials such as OAuth tokens are stored in `~/.hope-agent/credentials/`; they are cleared on sign-out |

**Dangerous mode (Global YOLO)** is a switch that skips all tool approvals in one click (Settings → Security Policy, or the command line `--dangerously-skip-all-approvals`); it is **extremely high risk—use it only in a fully trusted local environment**. It is orthogonal to Plan Mode—it only skips approvals and does not lift Plan Mode's restrictions. See [07 · Global YOLO](07-tools-and-permissions.md#75-full-auto-timeouts-and-unattended).

---

## 13.5 Reliability: keep-alive and crash recovery

Hope Agent uses multiple layers of redundancy to keep itself running reliably over the long term without dropping offline:

- **Guardian parent/child processes** (desktop): a supervisor process guards the main program and automatically restarts it with backoff after a crash.
- **System-service keep-alive**: after `hope-agent server install`, the operating system (launchd / systemd) brings it up.
- **Subsystem self-healing**: MCP reconnects automatically on disconnect, each IM account reconnects independently, and scheduled tasks recover idempotently.
- **Crash self-diagnosis**: after a certain number of consecutive crashes, it automatically runs a diagnosis once—first a full backup, then an analysis of the cause using the cheapest available model, followed by a safe in-place fix. **The auto-fix only touches configuration and clearly corrupted databases; it never touches your sessions, memory, skills, or Provider list.**

**Entry point**: Settings → System Health, where you can view crash records, diagnosis conclusions, applied fixes, and restore from full backups.

---

## 13.6 Reporting problems and self-diagnosis

You can ask the AI directly in conversation to organize a bug, feature request, or improvement into a GitHub issue draft and submit it, and you can also have it read the source code and logs to answer "how does Hope Agent implement X."

- **Entry point**: Settings → Tool Settings → Issue reporting (configure the target repository, GitHub token, etc.), or just say so in conversation.
- **It always confirms before submitting**; the title, body, and evidence are all redacted; the GitHub token never appears in the conversation, logs, or issue.

---

## 13.7 Other common settings

| Setting | Location | Details |
| --- | --- | --- |
| **Theme** | General → Appearance | Auto / Light / Dark, hot-switchable |
| **Interface language** | General → Appearance | Supports more than ten languages; note that "interface language" and "the reply language preference you tell the AI" (in Profile) are two different things |
| **Background animation** | General → Appearance | An all-weather background that can sync with weather effects |
| **Launch at startup / Prevent sleep** | General → System | Prevent sleep keeps the system awake (the display can still turn off) |
| **Global shortcuts** | General → System | Default "Quick Chat = Alt+Space," customizable; high risk (may conflict with the system) |
| **Proxy** | General → Network | Follow system / Direct / Custom; affects all outbound requests; high risk |
| **Notifications** | Notifications | Master switch, whether to include reply previews, background-task completion alerts, etc. |
| **Weather widget** | Tool Settings → General | City / geolocation, syncs with background animation |
| **Profile** | Profile | Avatar (croppable), name, time zone, AI experience level, reply style, etc., to help the AI fit you better |

---

## Closing

By now you have learned all of Hope Agent's main features and settings. If you still have questions:

- Return to the [table of contents](README.md) to search by topic.
- Ask the AI directly in the app—for example, "how do I set up a scheduled task"—it knows its own features.
- Report problems on [GitHub Issues](https://github.com/shiwenwen/hope-agent/issues), or chat on [Discussions](https://github.com/shiwenwen/hope-agent/discussions).

_This manual is continuously updated alongside the product; if anything differs from the actual interface, the in-app display takes precedence._
