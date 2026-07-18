# 10 · IM Channels

Connect Hope Agent to the instant-messaging tools you already use, and you can chat with your Agent directly on platforms like Telegram, Feishu, and WeChat, with the same capabilities as the desktop app (tools, memory, streaming replies). Images, voice, and files can also enter the multimodal context directly.

**In this chapter**

- [10.1 Which channels are supported](#101-which-channels-are-supported)
- [10.2 Adding an account](#102-adding-an-account)
- [10.3 Multimodal, approvals, and reply modes](#103-multimodal-approvals-and-reply-modes)
- [10.4 Session takeover and handover](#104-session-takeover-and-handover)
- [10.5 Slash commands in IM](#105-slash-commands-in-im)
- [10.6 Knowledge Space access](#106-knowledge-space-access)

---

## 10.1 Which channels are supported

Twelve channels are currently supported:

| Channel | Authentication | Notes |
| --- | --- | --- |
| **Telegram** | Bot Token | Most complete media support; typewriter-style streaming |
| **Discord** | Bot Token | DMs / servers / forums / channels |
| **Slack** | Bot Token + App Token | All media types |
| **Feishu / Lark** | App ID + App Secret | Supports card-style streaming |
| **QQ Bot** (official) | App ID + Client Secret | Requires a public URL in some scenarios |
| **WeChat** | QR-code login | DMs, encrypted media |
| **WhatsApp** | Bridge service URL + Token | Via a bridge |
| **Signal** | Phone number + linked device | Requires signal-cli |
| **iMessage** | Local macOS | **macOS only** |
| **LINE** | Channel Token + Secret | Requires a public URL in some scenarios |
| **Google Chat** | Service Account | Requires a public URL in some scenarios |
| **IRC** | Nick + NickServ | Plain text; media via download links |

> Channels without native media support automatically fall back to a plain-text "paste a download link" mode, so no message is lost.

---

## 10.2 Adding an account

**Entry point**: Settings → IM Channels → Add account → pick a channel → enter that channel's credentials (for example, a Bot Token for Telegram, which can be verified instantly).

In the account editor you can configure:

- **Bound Agent** — which Agent this account replies with by default.
- **DM policy** — Open (anyone can DM) / Allowlist (only specified users).
- **Group policy and user allowlist** — control who can use it.
- **Auto-approve tools**, **takeover notifications**, **startup online alerts**, **automatic voice transcription**, **Knowledge Space access**, **reply mode**, **thinking display**, and more.

---

## 10.3 Multimodal, approvals, and reply modes

### Multimodal

Channels that support media can send and receive images, video, audio, and files. Inbound media first passes a permission check, and only then is it actually downloaded and stored locally. Voice messages can enable ["automatic transcription"](02-models-and-providers.md#211-speech-to-text-stt) (off by default; consumes speech-recognition quota).

### Tool approvals

- Channels that support buttons (Telegram / Feishu / Discord / Slack / QQ / LINE / Google Chat) use **native button** approvals; other channels use **text-reply** approvals.
- **Auto-approve tools** (an account-level toggle, off by default): once enabled, tool approvals are skipped. If a skipped call would have hit the strictest approval, an audit warning is logged.

### Reply modes (imReplyMode)

All three modes apply to every channel; set them in the account editor, or switch with the `/imreply` command:

| Mode | Behavior |
| --- | --- |
| **Per round (split)** (`split`, default) | Each round's narration and media are sent as separate messages in chronological order; on streaming-capable channels each round is a genuine typewriter effect |
| **Final answer only** (`final`) | Sends only the last round's text, with media at the end, and does not enable streaming |
| **Streaming preview** (`preview`) | Streaming-capable channels render one continuously growing merged message; channels that don't support it automatically fall back to "Final answer only" |

The AI's thinking process is discarded by default; `/reason on` turns it on.

---

## 10.4 Session takeover and handover

Each IM chat (channel + account + chat + thread) is associated with only **one** session at any moment, and conversely each session can be taken over by only one IM chat (bidirectional 1:1).

- When a new chat takes over via `/session <id>` or a desktop handover, the old association is released, and the evicted chat receives a "this session has been taken over by another endpoint" notification (which can be silenced). After takeover, one already-completed round of reply is immediately backfilled.
- **Live streaming mirror**: a conversation you trigger on desktop or web, if that session is associated with an IM chat, is mirrored to the IM side in real time (rendered per the reply mode). The two channels are independent of each other — desktop always uses its own stream and is unaffected by the IM reply mode.
- Results from completed background jobs, Sub-Agents, and scheduled tasks are also posted back to the associated IM chat, or to the scheduled task's delivery targets.

---

## 10.5 Slash commands in IM

You can also use [slash commands](03-chat-and-sessions.md#39-slash-command-reference) in IM, though some behave differently:

| Command | What it does in IM |
| --- | --- |
| `/sessions` | Opens the session picker |
| `/session [<id>\|exit]` | With no argument, shows session info; `<id>` takes over; `exit` unbinds |
| `/project <name>` | Assigns the current session to a project |
| `/kb [on\|off]` | Confirms Knowledge Space access in group chats; in DMs, only reports status |
| `/imreply` / `/reason` | Reply mode / thinking display |
| `/status` | Session status, including the associated IM channel and Agent source |
| Others | Most of `/help`, `/new`, `/clear`, `/model`, `/search`, `/recap`, etc. are available |

> **Disabled in IM**: `/agent` (it easily causes Agent-ownership drift; change the binding in Settings instead) and `/handover` (it has no meaning on the IM side).

---

## 10.6 Knowledge Space access

IM channels have **zero Knowledge Space access** by default. Opening it up requires two layers of confirmation:

1. In desktop Settings, enable "Knowledge Space access" for the account (owner-only, off by default) — this covers DMs;
2. In group chats you must additionally confirm each one with `/kb on` inside the group.

Even when enabled, access is still bound by rules such as attachment, Incognito, and external read-only. See [05 · Knowledge Space · Access control](05-knowledge-space.md#58-access-control).

---

## Next steps

- Connect external tools (MCP) and customize behavior (Hooks) → [11 · Connect & Extend](11-connect-and-extend.md)
- Organize IM sessions into projects → [12 · Projects & Insights](12-projects-and-insights.md)
