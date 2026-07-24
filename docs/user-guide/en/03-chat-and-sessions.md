# 03 · Chat & Sessions

This chapter covers everyday use: how to use the chat interface, how to send images and voice, the various right-side panels, how to manage and search sessions, Incognito sessions, the session working directory, how long conversations are compacted automatically, and the full set of slash commands.

**In this chapter**

- [3.1 The chat interface](#31-the-chat-interface)
- [3.2 Sending attachments and multimodal input](#32-sending-attachments-and-multimodal-input)
- [3.3 @ mentions and input shortcuts](#33--mentions-and-input-shortcuts)
- [3.4 Thinking blocks, tool blocks, and the right-side panel](#34-thinking-blocks-tool-blocks-and-the-right-side-panel)
- [3.5 Managing and searching sessions](#35-managing-and-searching-sessions)
- [3.6 Incognito sessions](#36-incognito-sessions)
- [3.7 Session working directory](#37-session-working-directory)
- [3.8 Context compaction for long conversations](#38-context-compaction-for-long-conversations)
- [3.9 Slash command reference](#39-slash-command-reference)

---

## 3.1 The chat interface

- **Send**: `Enter` sends, `Shift+Enter` inserts a line break.
- **Stop**: while a reply is being generated, the send button turns into a square "Stop" button; click it to interrupt the current reply (equivalent to `/stop`).
- **Browse history**: with the cursor in an empty input box, press `↑` / `↓` to browse messages you sent earlier (much like terminal history).
- **Queue while busy**: if you send a message while the AI is still replying, it goes into a "pending queue"—you can edit it, delete it, or send it as the next standalone round. Whether queued messages are sent automatically once the AI is idle is controlled by the "Auto-send queued messages" setting (on by default).
- **Context usage bar**: a thin progress bar at the bottom of the input box (green → yellow → red) shows how much of the context window the current conversation is using; hover to see "used / total".
- **Quick switch**: `Shift+Tab` cycles quickly through the [permission modes](07-tools-and-permissions.md) (Default → Smart Approval → YOLO).

---

## 3.2 Sending attachments and multimodal input

Hope Agent supports three kinds of input—images, files, and voice—all handed to the model together with your message.

- **Images / files**: click the "+" button in the input toolbar to choose them, or simply **paste / drag and drop**.
- **Pasting long text**: pasting a large block of text automatically turns it into a text attachment, so it doesn't overflow the input box.
- **Voice**: click the microphone button to record and transcribe; you can also **hold `Ctrl+Shift+H` and speak** inside the app window, then release to transcribe automatically and insert the text at the cursor. While recording, a waveform and a live elapsed timer are shown; transcription services that support streaming produce text as you speak.

> The voice feature requires you to first [configure Speech-to-Text (STT) in Settings](02-models-and-providers.md#211-speech-to-text-stt); if it isn't configured, you'll be prompted. The default limit is 20 MB per file, and up to 64 files per message.

---

## 3.3 @ mentions and input shortcuts

A few special characters in the input box let you reference things quickly:

| Input | What it does |
| --- | --- |
| `/` | Triggers a [slash command](#39-slash-command-reference) |
| `@` | Mentions a **skill** (inserts a rose-pink skill chip and injects the skill's instructions into this round when sent) or **a file in the current working directory** |
| `[[ ]]` | Mentions a [Knowledge Space note](05-knowledge-space.md); the note's content is injected when sent |
| `#` | Inserts one of your preset **quick prompts** (reusable prompt snippets) |

> - `@skill` mentions are only available for a small set of fixed built-in skills (the office trio, data analysis, browser control, macOS control), and in the main chat you must first enable "Skill mentions" in Settings (off by default).
> - The content injected by `[[note]]` note mentions and `@` file mentions is treated as "untrusted external data" and is never interpreted as instructions to the AI.

---

## 3.4 Thinking blocks, tool blocks, and the right-side panel

### Thinking blocks

The model's reasoning is shown as a collapsible "Thinking block" with a brain icon and a **live elapsed timer** (refreshed every 100 ms while streaming, then showing the final time when finished). You can click to collapse / expand it; whether it is expanded by default is controlled by the "Auto-expand the thinking process" setting.

### Tool call blocks

Every time the AI calls a tool (reading a file, running a command, searching, etc.) it shows a block with the tool name, arguments, result, **elapsed time**, and success / failure status. Several consecutive completed tool steps are automatically collapsed into one "Processed" group to keep the interface tidy.

Some special tools have their own cards, such as the question card (`ask_user_question`), the plan card, the task checklist, the skill-activation block, the sub-agent block, and so on.

### The right-side panel

The right side of the chat interface shows only one context panel at a time (they are mutually exclusive); you can drag the edge to widen it, or maximize it:

- **File Diff panel**—shows the AI's changes to files (split / unified view); see [07 · Tools & Permissions](07-tools-and-permissions.md).
- **Workspace**—aggregates the current task's progress, the files read and modified, the URLs visited, Git status, goals, and so on; open it from the status bar above the input box.
- **Plan panel**—the design document under [Plan Mode](08-autonomous-tasks.md#85-plan-mode).
- **Browser / macOS control / Canvas / Team / Background jobs / File browser / File preview / Pull Request**—each corresponds to its own feature; see the relevant chapters.

### Message rendering

Replies support full Markdown, syntax-highlighted code, math formulas (KaTeX), and Mermaid diagrams. In the desktop app, the AI can also write clickable local path links (clicking opens them in the file manager).

### Related settings (Settings → Chat & Context → Basics)

| Setting | Default | What it does |
| --- | --- | --- |
| Auto-send queued messages | On | Whether messages you send while the AI is busy are queued and sent automatically |
| Auto-expand the thinking process | On | Whether Thinking blocks are expanded by default while streaming |
| Collapse intermediate messages when done | On | Whether completed rounds are collapsed automatically |
| Progress updates | On | Whether the AI gives brief progress updates at key moments and a short summary at the end of a round |
| Session title | On | Whether the model automatically titles sessions |

---

## 3.5 Managing and searching sessions

- **New conversation**: "New" in the sidebar, or `/new`. When you start a new conversation inside a project, it stays as a draft first and is only saved once you send the first message.
- **Session list**: the sidebar is sorted by most recently updated and can be pinned. It has two browsing tabs, "Sessions" and "Sub-Agents". Sessions for scheduled tasks, Incognito, the Knowledge Space, and the Design Space **do not appear in the main list**.
- **Switch / rename / archive**: click to switch; you can rename manually (a manual name won't be overwritten by the auto title). Archiving hides a conversation from everyday lists and search while retaining its messages, project, and Agent association.
- **Restore / permanently delete**: in **Settings → Archived conversations**, search or filter by type and project, then restore any conversation. Permanent deletion is available only there and asks for confirmation; it also removes messages and attachments and cannot be undone.
- **Continue in a new session (Fork)**: copies the current session into a new, independent session (copying the conversation content and configuration, but not any running goal, ongoing progression, or background jobs).

### Search

- **Global search** (sidebar): full-text search across all sessions, with matched snippets highlighted.
- **In-session search** (`Cmd+F` / `Ctrl+F`): searches only the current session, scrolling to and highlighting each match.

### How unread is counted

- **No matter how many unread messages a session has, it counts as just "1 unread session"**. A session row shows a small dot; the numbers shown for projects, globally, and on the Dock are the **count** of unread sessions; the system tray only shows whether there is a red dot at all.
- A session is considered "read" only when it is selected in the main chat view, the window is in the foreground, the page is visible, and the message list is at the latest position.
- Scheduled tasks and IM channels each keep their own separate unread counts; "Mark all as read" for ordinary conversations does not clear them.

---

## 3.6 Incognito sessions

An Incognito session is a **temporary conversation that leaves no trace**, burned on close.

**How to turn it on**: click the **ghost icon** in the input toolbar or the title bar.

An Incognito session does the following to protect your privacy:

- It is deleted completely the moment you switch away from it, leaving no record.
- **It does not inject memory, does not do behavior awareness, does not extract memory automatically, and does not participate in Knowledge Space passive recall**—the memory system is entirely "blind" to it.
- It does not appear in the sidebar list, global search, or Dashboard statistics.

> **Limitations**: Incognito sessions are mutually exclusive with "projects", "IM channels", "Workflow mode", and "an in-progress goal"—a session already assigned to a project or bound to IM cannot be switched to Incognito (the toggle is grayed out with an explanation). You can still set a session working directory within an Incognito session.

---

## 3.7 Session working directory

Assign a default working directory to the current conversation. It will:

- be injected into the AI's prompt, telling it where to read and write files by default;
- act as the actual working directory for `exec` (running commands) and as the resolution root for relative paths in `read` (reading files);
- have its top-level file listing shared with the AI, so it can sense what you're working on.

**How to set it**: click the "Working directory" button in the input toolbar. The desktop app opens the system directory picker; the web / server version opens a directory browser (pointing at a path on the server machine).

The priority is "session setting > project setting > default workspace". A session inside a project always has a working directory (by default `~/.hope-agent/projects/{project}/workspace/`).

---

## 3.8 Context compaction for long conversations

The amount of conversation a model can remember is limited (the context window). As a conversation grows and approaches the limit, Hope Agent **compacts it automatically in layers**, so long conversations and long tasks can continue without suddenly "losing memory" or erroring out. Compaction happens automatically; you'll usually just see a compaction notice at the top.

Compaction starts with "cleaning up stale tool results at zero cost" and works its way up to "summarizing old history"; after summarizing, it also automatically re-injects the current contents of recently edited files, so the AI doesn't forget the file it's working on.

**Manual commands**:

- `/compact`—compact the current conversation manually.
- `/context`—view a breakdown of current context usage.

**Main settings (Settings → Chat & Context → Context compaction)**:

| Setting | Default | What it does |
| --- | --- | --- |
| Enable automatic context compaction | On | When off, only the most basic truncation and fallback are kept |
| Reactive micro-compaction | On | Cleans up old tool results as needed at the end of each round |
| Trigger ratio | 0.75 | Reactive cleanup is triggered when context usage exceeds this ratio |
| Summarization model | Empty (uses the chat model) | The model used to generate summaries; you can specify a cheaper model to save cost |
| Summarization trigger threshold | 0.85 | Summary-based compaction is triggered when usage exceeds this ratio |

> These are "behavior adjustment" settings (medium risk) that affect context and cost. The vast majority of users can keep the defaults. Compaction also has a full set of finer parameters (trim ratio, number of recent rounds to keep, file recovery, and so on), all in the "Advanced" area of the same panel—adjust them as needed.

---

## 3.9 Slash command reference

Type `/` in the chat input box or in any IM channel to trigger a slash command. Fuzzy matching is supported. Below is the full set of built-in commands (each installable skill also auto-generates a `/skillname` command).

### Sessions

| Command | Purpose |
| --- | --- |
| `/new` | Create a new session |
| `/clear` | Delete all messages in the current session |
| `/compact` | Compact the current session's context |
| `/stop` | Stop the current reply |
| `/rename <title>` | Rename the session |
| `/export` | Export the current session as Markdown |
| `/status` | View session status (Agent / model / ID / message count) |
| `/usage` | View the current session's token usage |
| `/context` | View a breakdown of context-window usage |
| `/sessions` | Open the session picker (optionally with a keyword search) |
| `/session <id>` | Switch to / associate a session |

### Projects and goals

| Command | Purpose |
| --- | --- |
| `/project [name]` | Open the project picker, or start a new conversation in a project |
| `/projects` | List all projects |
| `/goal <goal> --criteria <completion criteria>` | Set / manage the session goal; see [Chapter 08](08-autonomous-tasks.md#81-goal) |
| `/plan` | Enter / manage [Plan Mode](08-autonomous-tasks.md#85-plan-mode) |
| `/workflow [on\|off\|ultracode]` | Toggle [Workflow mode](08-autonomous-tasks.md#82-workflow) |
| `/mode <off\|guarded\|deep\|autonomous>` | Set the [Execution Mode](08-autonomous-tasks.md#84-execution-mode) |
| `/loop <instruction>` | Create / manage an [ongoing-progression task](08-autonomous-tasks.md#83-loop) |

### Models and thinking

| Command | Purpose |
| --- | --- |
| `/model [name]` | List or switch models |
| `/models` | List all available models |
| `/thinking <off\|low\|medium\|high\|xhigh>` | Set the thinking effort (alias `/think`) |

### Memory

| Command | Purpose |
| --- | --- |
| `/remember <content>` | Save a memory |
| `/forget <keyword>` | Search for and delete the best-matching memory |
| `/memories` | List memories |

### Agents and teams

| Command | Purpose |
| --- | --- |
| `/agent <name>` | Switch Agent (creates a new session automatically; disabled in IM) |
| `/agents` | List all Agents |
| `/team` | Manage [Agent Teams](09-multi-agent-and-scheduling.md#92-agent-teams) |

### Permissions and tools

| Command | Purpose |
| --- | --- |
| `/permission <default\|smart\|yolo>` | Set the [permission mode](07-tools-and-permissions.md#72-three-permission-modes) |
| `/review` | Run a local review of uncommitted code changes |
| `/search <query>` | Ask the AI to search for you |
| `/recap [--range=7d\|--full]` | Generate a [Recap report](12-projects-and-insights.md#124-recap-reports) |
| `/awareness <off\|on\|mode ...>` | Toggle [behavior awareness](04-memory.md#46-behavior-awareness) |
| `/prompts` | View the current Agent's full system prompt |
| `/help` | Show all commands |

### IM-channel only

| Command | Purpose |
| --- | --- |
| `/imreply <split\|final\|preview>` | Set the IM reply mode |
| `/reason <on\|off>` | Whether IM output includes the thinking process |
| `/kb <on\|off>` | Confirm [Knowledge Space access](05-knowledge-space.md) in a group chat |
| `/handover <channel:account:chat>` | On the desktop, push the session to an IM conversation |

> For more differences in how IM-related commands behave, see [10 · IM Channels](10-im-channels.md).

---

## Next steps

- Make the AI remember your preferences → [04 Memory](04-memory.md)
- Control the AI's operating permissions → [07 Tools & Permissions](07-tools-and-permissions.md)
- Let the AI drive tasks in the background over time → [08 Autonomous Tasks](08-autonomous-tasks.md)
