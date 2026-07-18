# 07 · Tools & Permissions

Hope Agent "gets work done" because the AI can call **tools** to actually operate: run commands, read and write files, search the web, and control the browser and computer. So that you can hand it these capabilities with confidence, every sensitive operation goes through **approval**. This chapter explains which tools exist, how to govern permissions, and how the sandbox, browser control, and computer control work.

**In this chapter**

- [7.1 The built-in toolbox at a glance](#71-the-built-in-toolbox-at-a-glance)
- [7.2 Three permission modes](#72-three-permission-modes)
- [7.3 The approval dialog](#73-the-approval-dialog)
- [7.4 Protected paths, dangerous commands, and edit commands](#74-protected-paths-dangerous-commands-and-edit-commands)
- [7.5 Full-auto, timeouts, and unattended](#75-full-auto-timeouts-and-unattended)
- [7.6 Docker Sandbox](#76-docker-sandbox)
- [7.7 Browser control](#77-browser-control)
- [7.8 Computer control (macOS)](#78-computer-control-macos)
- [7.9 File operations, Git, and isolated worktrees](#79-file-operations-git-and-isolated-worktrees)

---

## 7.1 The built-in toolbox at a glance

The AI calls these tools **automatically** when needed; you usually don't trigger them by hand, but you can govern them in the permission settings. Here are the main tools (by category):

| Category | Tool | What it does |
| --- | --- | --- |
| **Commands** | `exec` | Run shell commands (has its own command-level approval; supports background execution, timeouts, sandbox) |
| | `process` | Manage background-running commands (view output, write input, terminate) |
| **Files** | `read` / `write` / `edit` / `apply_patch` | Read / write / precisely modify / batch-patch (write operations require approval) |
| | `ls` / `grep` / `find` | List directories / search content / find files by pattern |
| | `lsp` | Semantic code intelligence (definitions / references / diagnostics, etc.) |
| **Network** | `web_search` / `web_fetch` | Web search / fetch a web page's main content |
| **Multimodal** | `image` / `pdf` | Let the model view images / parse PDFs |
| | `image_generate` | AI image generation |
| **Browser / Computer** | `browser` | Drive the browser (see [7.7](#77-browser-control)) |
| | `mac_control` | Control the macOS desktop (see [7.8](#78-computer-control-macos)) |
| **Memory / Knowledge** | Memory and note tools | Save / recall memory, read and write notes (called internally, no approval dialog) |
| **Collaboration** | `subagent` / `team` / `acp_spawn` | Invoke a Sub-Agent / team / external ACP Agent |
| **Control plane** | `task_*` / `goal_*` / `workflow` / `loop_*` | Task progress, goals, workflows, continuous execution |
| **Interaction** | `ask_user_question` | Ask you structured questions |
| | `send_notification` / `send_attachment` | Send system notifications / push downloadable files |
| **Others** | `canvas` / `get_weather` / `manage_cron` / `schedule_wakeup`, etc. | Visualization sandbox / weather / scheduled tasks / self-wakeup |

**Common tool settings (Settings → Tool Settings → General):**

| Setting | Default | Effect |
| --- | --- | --- |
| Tool timeout | 0 (unlimited) | Timeout in seconds for a single tool execution |
| Tool-result disk threshold | 50 KB | Beyond this, results are written to disk and only a head/tail preview is kept in context |
| Deferred tool loading | Recommended (on by default) | Defers a batch of low-frequency / large-schema tools by default, discovered on demand via tool_search (saves context) |
| Max images / PDFs / vision pages | 10 / 5 / 10 | Per-call limits |

---

## 7.2 Three permission modes

Each session independently picks one permission mode, which decides whether tool calls trigger approval. **How to switch**: the permission-mode switcher on the chat input toolbar, the `/permission` command, or `Shift+Tab` to cycle quickly.

| Mode | Behavior | Who it's for |
| --- | --- | --- |
| **Default** | Edit operations force approval, plus any tools you checked as custom for the Agent | Most users |
| **Smart Approval** | The AI decides: highly confident calls are auto-allowed, or a "judge model" decides; re-editing a file you've already edited is auto-allowed | Advanced users who trust the AI's judgment |
| **YOLO** | Everything in that session is allowed (only Plan Mode can still block) | One-off scripts, extremely trusted scenarios |

**About Smart Approval**: it has three strategies—rely only on the AI's self-reported confidence, use the judge model directly, or combine the two. **Smart Approval ignores your "custom tool approval" checkboxes** (the UI reminds you of this). But protected paths, dangerous commands, low-level browser commands, external-service write operations, and the like are already blocked before the smart judgment runs, so they are unaffected.

> **Plan Mode has the highest priority**—even in full-auto mode, [Plan Mode](08-autonomous-tasks.md#85-plan-mode) can block things that shouldn't be done.

---

## 7.3 The approval dialog

When a tool needs confirmation, an approval dialog pops up with three choices:

- **Deny**
- **Allow Once**
- **Allow Always**—remembers this decision and no longer asks for the same kind

The dialog shows the reason for the operation, the working directory, a command / operation summary, and a countdown ring (if you've enabled the [approval timeout](#75-full-auto-timeouts-and-unattended)).

> **Some operations can't be "Allow Always"** (they must be confirmed manually every time)—including protected paths, dangerous commands, high-risk macOS control, low-level browser commands, external-service write operations, and questions asked in Plan Mode. These are the most sensitive operations, and the "approve once and for all" option is deliberately withheld.
>
> **Consistent across endpoints**: the same approval can appear simultaneously on desktop, web, and IM. Once any endpoint handles it, the others uniformly dismiss it and note "handled by another endpoint."

---

## 7.4 Protected paths, dangerous commands, and edit commands

These are three editable lists; a match forces approval. **Entry**: Settings → Permission.

| List | Trigger | Examples | Can "Allow Always"? |
| --- | --- | --- | --- |
| **Protected paths** | Read / write / edit / execute | `~/.ssh/`, `~/.aws/`, `/etc/`, `.env`, `*secret*`, `*.pem`, `*.key` | No |
| **Dangerous commands** | Execute a command | `rm -rf /`, `git push --force`, `git reset --hard`, `mkfs`, `DROP TABLE` | No |
| **Edit commands** | Execute a command | `rm `, `mv `, `git commit`, `npm install`, `> `, `>>` | Yes (Default mode only) |

You can add and remove entries in these three lists, or "Restore defaults." These settings are **high risk** (permission rules).

> In addition, Feishu write tools and write operations of external connectors (Gmail / Calendar / Drive / Slack / Notion, etc.) are automatically recognized as the strictest approval; they don't go into these lists, and they can't be "Allow Always."

---

## 7.5 Full-auto, timeouts, and unattended

### Global YOLO (skip all approvals)

Skip **all** tool approvals with one switch—extremely high risk, use only in a fully trusted local environment.

- **Entry**: Settings → Security Policy → Dangerous Mode (enabling requires checking "I understand the risks"), or start from the command line with `--dangerously-skip-all-approvals`.
- Once on, running commands, writing / editing files, browser operations, and all IM-triggered tool calls are allowed; when a protected path / dangerous command is matched, only an audit-log entry is recorded—no dialog. Plan Mode's restrictions still apply.

### Approval timeout

**Never times out by default**—an approval waits for you indefinitely. You can enable a timeout in Settings → Permission:

| Setting | Default | Effect |
| --- | --- | --- |
| Auto timeout | Off (never times out) | Whether the timeout is enabled |
| Timeout seconds | 300 seconds | Wait duration (0 = unlimited) |
| Timeout action | Deny | On timeout: deny / proceed |

> **Important**: "proceed on timeout" only applies to ordinary operations; the most sensitive operations (protected paths, dangerous commands, etc.) are **always force-denied on timeout** even if you set them to proceed.

### Unattended

When a tool needs approval but there truly is nobody to respond (for example, a scheduled task running automatically in the small hours, or a headless server), it is **denied** by default (Settings → Permission → Unattended). As long as someone might be present (a desktop window, a web client, or IM), it is treated as attended and pops the dialog normally—only scenarios proven to have nobody, such as scheduled tasks, follow the unattended policy.

---

## 7.6 Docker Sandbox

The sandbox runs `exec` commands inside a Docker container rather than directly on your computer, protecting the host machine.

**How to use**: the "Sandbox" section in the permission-switcher popover on the chat input area (session-level), or set a default sandbox mode in the Agent settings.

| Mode | Where it runs | File writes |
| --- | --- | --- |
| Off (default) | Your computer | Take real effect |
| Standard | Docker, mounting the current directory | Land in the real workspace |
| Isolated | Docker + a temporary copy | Land only in the temporary copy, deleted on exit |
| Workspace | Docker mounting the workspace | Land in the real workspace (approvals relaxed for commands inside the workspace) |
| Trusted | Maximum autonomy inside Docker | Same as Workspace (the most sensitive operations are still approved every time) |

> **If you pick a sandbox but Docker is unavailable, tool execution fails outright (with an error)—it never silently falls back to running on your computer.** The sandbox does not bypass permissions—the most sensitive operations are still approved every time.

**Container configuration (Settings → Docker Sandbox)**: image (default `debian:bookworm-slim`), memory (512 MB), CPU (1 core), read-only root filesystem (on), network mode (default `none`, no network), and more. The top of the panel shows whether Docker is available; if it's not installed / not running, it gives platform-specific install guidance.

---

## 7.7 Browser control

The AI drives the browser through the `browser` tool—it can open web pages, click, fill forms, scrape content, take screenshots, and more.

**Two backends**:

- **Chrome extension (default)**—controls **your actual signed-in Chrome** through a Chrome extension, carrying all of your cookies and login state, and can directly operate already-open tabs.
- **Standalone browser (fallback / for Docker)**—uses a separate Chromium instance, isolated from your real Chrome data.

The backend preference can be chosen in Settings → Browser: **Extension first (default)** / **Isolated CDP only** / **Extension only**.

**Live mirror** (desktop only): there's a browser panel on the right of the chat that mirrors the page the AI is operating in real time, so you can see what it's visiting and clicking.

**Security**:

- Sending **raw DevTools commands** (low-level browser commands) is the strictest operation—**approved every time and never "Allow Always"**; there's also a hard switch to completely forbid the AI from sending such commands.
- Executing arbitrary JavaScript, taking over your real tabs, reading real downloads, and the like all trigger approval.
- Every navigation URL passes a security check (to prevent access to internal networks).
- For sign-in, two-factor verification, and CAPTCHAs, the AI always hands them off to you by asking.

---

## 7.8 Computer control (macOS)

> Truly usable only on the **macOS desktop version**. Web / server mode and other platforms are not supported.

The AI reads the screen and accessibility information through the `mac_control` tool, operating apps, windows, menus, the clipboard, and dialogs.

**Grant permissions first** (Settings → Permissions, macOS only):

- **Accessibility**—read UI elements, click, type, control windows and menus;
- **Screen Recording**—screenshots, the right-side mirror panel, visual locating.

The panel shows the current authorization status and missing permissions; clicking "Authorize" pops the system dialog, and clicking "Check" jumps to System Settings.

**What it can do**: read-only actions (view the foreground app / windows / element tree, screenshot for the model to view, find elements, list apps / windows / menus) are generally allowed directly; operation actions (click, type, paste, keyboard shortcuts, drag, switch app / window / desktop, click menus, read and write the clipboard) require approval; **high-risk actions** (quit an app, close a window, click dangerous buttons like "Delete / Empty / Reset") can't be "Allow Always" and must be confirmed every time.

> The AI does not read the real contents of password fields, does not stuff screenshots into the context, and cannot control Hope Agent's own window in the background. Before the approval dialog it remembers the current foreground app and restores it after approval, to avoid stealing focus.

---

## 7.9 File operations, Git, and isolated worktrees

### File operations

Opening / downloading / previewing a file is decided by **the machine the file lives on**:

- **Local desktop**—open with the system default app, and you can reveal it in the file manager.
- **Web / remote**—download within the browser / app.

Previewable types (code, text, Markdown, images, PDFs, audio/video, Office documents) can be viewed directly in the right-side preview pane. Files are editable in the UI (valid text, within 5 MB), and saving has conflict detection (it only offers reload / save as / cancel—it never force-overwrites).

> In web / server mode, write operations on server files are governed by the "allow remote writes" switch (off by default); the local desktop is unrestricted.

### Git control

When the session is inside a Git repository, a Git control card appears in the workspace: review diffs (stage / discard by file or by hunk), switch / create branches, stage a commit, push, and create and manage GitHub PRs (check status, review, merge).

> Git control **does not automatically fetch / stash / pull / rebase, nor does it offer force push**—to avoid accidentally overwriting your work. External text inside a PR is treated as untrusted data.

### Isolated worktrees

Long tasks, [Workflows](08-autonomous-tasks.md#82-workflow), and [Sub-Agents](09-multi-agent-and-scheduling.md#91-sub-agents) can modify code in a **separate directory** without polluting your main workspace. When creating a Workflow you can choose "where to run" (current directory / a new isolated worktree / an existing worktree); user-delegated Sub-Agents try to create an isolated worktree by default. Migration between the main tree and a worktree fully copies the changes and verifies them, rolling back automatically on failure.

---

## Next steps

- Have the AI push tasks forward in the background over time → [08 Autonomous Tasks](08-autonomous-tasks.md)
- Multi-Agent collaboration and scheduled tasks → [09 Multi-Agent & Scheduled Tasks](09-multi-agent-and-scheduling.md)
- Understand all the security boundaries → [13 Settings & Security](13-settings-and-security.md)
