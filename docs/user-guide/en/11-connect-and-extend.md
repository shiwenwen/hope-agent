# 11 · Connect & Extend

This chapter covers three ways to extend Hope Agent's capabilities: connect external tools with **MCP**, insert custom handling at key moments with **Hooks**, and equip the AI with reusable specialized methodology through **skills**.

**In this chapter**

- [11.1 MCP: connecting external tools](#111-mcp-connecting-external-tools)
- [11.2 Using Hope Agent as an MCP server](#112-using-hope-agent-as-an-mcp-server)
- [11.3 Hooks: lifecycle hooks](#113-hooks-lifecycle-hooks)
- [11.4 The skill system](#114-the-skill-system)

---

## 11.1 MCP: connecting external tools

MCP (Model Context Protocol) is an open standard that lets AI assistants connect to all kinds of external "tool servers." Hope Agent ships with a full MCP client that can inject the tools, resources, and prompts offered by external servers directly into the conversation.

- **Four connection methods**: stdio, Streamable HTTP, SSE, WebSocket.
- **Full OAuth 2.1 support**: servers that require authorization can go through OAuth login automatically, with credentials stored securely on disk.

**How to add one** (Settings → MCP Servers):

- The left column is the server list (with status, connection method, and tool count); the right column is the editor. You can configure the name, enabled state, trust level, connection method, tool allow/deny lists, timeout, and auto-approval (which can only be enabled for servers whose trust level is "Trusted").
- Click "Test Connection" to see the result and tool count in real time; servers that require authorization show an "Authorize" button that triggers OAuth.
- Supports one-click import from `claude_desktop_config.json`.

> MCP server configuration is **high risk**—it may carry credentials. The server configuration itself (including keys and environment variables) can only be changed in the interface; the AI cannot modify it through conversation. All tools are injected by default; for large servers with many tools, you can enable "lazy loading" to switch to on-demand discovery and avoid consuming context.

---

## 11.2 Using Hope Agent as an MCP server

Conversely, Hope Agent can also act as an **MCP server**, letting other AI tools on the machine (Claude Code, Cursor, etc.) tap into its capabilities.

- **Entry point**: run `hope-agent mcp` on the command line.
- **Read-only** by default; only with `--allow-writes` does it expose write tools.
- For safety, it **never exposes** these capabilities: writing to your code repository, deploying, sharing, deleting, or exporting.

---

## 11.3 Hooks: lifecycle hooks

Hooks let you attach custom handlers at key moments (before/after tool calls, session start/end, context compaction, permission decisions, and more). They align field-for-field with Claude Code's hooks protocol, so community scripts work out of the box.

**Entry point**: Settings → Hooks. Pick an event, add matching rules, then add a handler.

**Five handler types**:

| Handler | What it does |
| --- | --- |
| command | Run a shell command |
| http | POST to a URL |
| mcp_tool | Call an MCP tool |
| prompt | Make a one-shot LLM call, with the result used as additional context |
| agent | Spawn a sub-Agent |

**Four scope layers** (all stack together):

- **User / managed**—apply globally.
- **Project / local**—apply per session working directory, **disabled by default** (supply-chain protection); you must explicitly enable "Allow project & local hooks" in settings.

| Setting | Default | What it does |
| --- | --- | --- |
| Disable all hooks | Off | Turn off all hooks in one click |
| Allow project & local hooks | Off | Whether hooks checked into a repository take effect per directory |

> Hooks are a **high-risk** feature (they can run arbitrary commands), so **the AI can only read hook configuration, not change it**—it can only be edited in the interface, preventing the AI from equipping itself with command-execution capabilities.

---

## 11.4 The skill system

A skill equips the AI with **specialized methodology / an operating manual**—a directory plus a `SKILL.md` file. The system prompt only holds each skill's "name + description"; the AI activates skills on demand, so they don't permanently take up context.

### Built-in skills

Hope Agent includes a set of skills, organized by category:

- **Platform self-management**: `ha-settings` (change settings by conversation), `ha-skill-creator` (create / edit skills), `ha-find-skills` (discover and install third-party skills), `ha-browser` (browser automation methods), `ha-mac-control` (macOS control methods, macOS only), `ha-knowledge` (Knowledge Space workflow), `ha-logs` / `ha-data-stores` (read-only inspection of local data), `ha-self-diagnosis` (self-diagnosis + filing a GitHub issue), `ha-self-update` (check for and install updates).
- **Programming methodology**: eight of them—implementation, planning, debugging, test strategy, code review, multi-Agent collaboration, completion verification, workflow authoring, and more (recommended and combined automatically per task).
- **Office trio** (requires python3): `office-docx` (Word), `office-xlsx` (Excel), `office-pptx` (PowerPoint)—create, edit, review, and deliver, with support for real tables, charts, formulas, comments and tracked changes, and more.
- **Office methodology**: meeting minutes, email drafting, weekly/monthly reports, chart drawing.
- **Integrations**: `feishu` (over 30 tools spanning Feishu cloud documents / Bitable / cloud drive / knowledge base / approvals / calendar / contacts / recruiting, and more), `ha-data-analytics` (local data analysis + generating shareable reports).

### Three activation methods

| Method | How to use it | Difference |
| --- | --- | --- |
| **AI-driven** | The AI calls the `skill` tool | The main entry point. Ordinary skills inject their instructions into the main conversation; skills marked `fork` run in a separate sub-Agent and only return a summary (keeping the main conversation clean) |
| **Slash command** | Type `/skillname [args]` | You trigger it yourself |
| **`@skill` mention** | Pick a skill from the `@` menu in the input box | Open only to a small, fixed set of built-in skills (the office trio, data analytics, browser, macOS control); the main conversation must have "skill mentions" enabled first |

### Authoring and installing

- **Authoring**: use the `ha-skill-creator` skill, or create one under "Settings → Skills." Skills the AI creates on its own enter a **draft** state; they don't take effect immediately and wait for your review (passing through a security scan and other gates).
- **Installing third-party skills**: `ha-find-skills` searches and discovers skills from ClawHub, Skillhub (better suited to users in mainland China), or GitHub. **Installing third-party skills is a high-risk operation** (third-party code enters the AI's toolchain), so the AI must first show you the candidates and get your **explicit confirmation** before installing.
- **Compatibility standard**: follows the [agentskills.io](https://agentskills.io) open standard, and is also compatible with skills from ecosystems like Claude Code and OpenAI Codex, which can be imported directly.

---

## Next steps

- Organize your work, review it, and manage costs → [12 · Projects & Insights](12-projects-and-insights.md)
- Adjust settings by conversation → [13 · Settings & Security](13-settings-and-security.md)
