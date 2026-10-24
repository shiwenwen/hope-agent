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

Thirteen channels are built in, with the capabilities listed below:

| Channel | Authentication | Notes |
| --- | --- | --- |
| **Telegram** | Bot Token | Most complete media support; typewriter-style streaming |
| **Discord** | Bot Token | DMs / servers / forums / channels |
| **Slack** | Bot Token + App Token | All media types |
| **Feishu / Lark** | App ID + App Secret | Supports card-style streaming |
| **QQ Bot** (official) | App ID + Client Secret | Requires a public URL in some scenarios |
| **WeChat** | QR-code login | DMs, encrypted media |
| **WeCom** | Bot ID + Secret | Direct AI-bot DM/group final text; first-version limits below |
| **WhatsApp** | Bridge service URL + Token | Via a bridge |
| **Signal** | Phone number + linked device | Requires signal-cli |
| **iMessage** | Local macOS | **macOS only** |
| **LINE** | Channel Token + Secret | Requires a public URL in some scenarios |
| **Google Chat** | Service Account | Requires a public URL in some scenarios |
| **IRC** | Nick + NickServ | Plain text; media via download links |

> The first WeCom version receives and replies with text only; it does not forward media or media links. Other channels without native media support automatically fall back to a plain-text "paste a download link" mode, so no message is lost.

---

## 10.2 Adding an account

**Entry point**: Settings → IM Channels → Add account → pick a channel → enter that channel's credentials (for example, a Bot Token for Telegram, which can be verified instantly).

Telegram also accepts an optional **Bot API Base URL**. You can enter the root URL of a trusted HTTPS reverse proxy you operate (for example, `https://tg.example.com`); leave it empty to keep using the official API. Enter only the root—do not append `/bot<TOKEN>/getMe`. The proxy must forward both `/bot…` API requests and `/file/bot…` media downloads. A reverse proxy can see the Bot Token, messages, and media, so do not use an untrusted public proxy.

**WeCom text first version:** obtain Bot ID and Secret from the WeCom AI-bot administration page and enter them only in Channel Settings. Use one client per bot. Validation briefly subscribes, or reuses an existing healthy local connection; do not run another client concurrently. Stop the running account before validating a changed Secret for the same bot. Validation of an unsaved or cancelled credential draft does not open a second connection that replaces the running account.

New accounts default to DM/group allowlists; empty lists grant no access. Enter enterprise member `userid` values for DMs. Enter one group `chatid` (or `group:chatid`) per line; explicitly adding a new group allows its text without @mentions. Existing group policies remain intact. The verified protocol has no mention flag, so existing mention-only groups stay blocked. Local channel denial logs contain sanitized sender/chat IDs for setup; do not copy message contents or Secret values.

The first version sends one final text reply per original connection callback, at most **20 KiB UTF-8**. Media, cards, native previews, proactive notifications and desktop mirror backfills are unavailable. Approval/question notifications without a callback anchor cannot be sent here; handle them in the app. A missing answer never counts as approval. Network failures reconnect with bounded backoff; rejected credentials require correction and explicit validation. After another client replaces the connection, stop that client before restarting this account. Unknown acknowledgements are never retried automatically: inspect the result in WeCom first. Expired (10-minute), capacity-evicted or reconnected anchors require a new inbound message. Validation covers synthetic local protocol fixtures, with no real enterprise-tenant acceptance yet.

In the account editor you can configure:

- **Bound Agent** — which Agent this account replies with by default.
- **DM policy** — Open (anyone can DM) / Allowlist (only specified users).
- **Group policy and user allowlist** — control who can use it.
- **Auto-approve tools**, **takeover notifications**, **startup online alerts**, **automatic voice transcription**, **Knowledge Space access**, **reply mode**, **thinking display**, and more.
- Telegram/Slack can disable native streaming per account. iMessage, Google Chat, and Discord expose protocol or rollout switches and fall back to their compatibility paths when disabled.
- iMessage health checks show the reported imsg version. Versions below 0.15.8 prompt a manual update; older builds without a version show “unverified” and are neither upgraded nor blocked automatically.
- Signal and WhatsApp account cards show detected sidecar/Bridge versions. Unknown versions are warning-only; a known-vulnerable Baileys Bridge is blocked at startup and before sends.
- Signal reception preserves Chinese text and emoji across network chunks and supports multiline SSE messages. Stopping an account cancels connections and queue waits. Invalid events or events exceeding 1 MiB disconnect and reconnect; disconnected-message replay is not yet supported.

---

## 10.3 Multimodal, approvals, and reply modes

### Multimodal

Channels that support media can send and receive images, video, audio, and files. Inbound media first passes a permission check, and only then is it actually downloaded and stored locally. Voice messages can enable ["automatic transcription"](02-models-and-providers.md#211-speech-to-text-stt) (off by default; consumes speech-recognition quota).

An Agent can also explicitly request one PDF, TXT, or MD file in IM (one file, up to 10 MiB). Discord can open a native upload modal when enabled for the account; other channels ask you to send the next attachment. The file is bound only to the exact account, chat, thread, and session that received the question, so expired or mismatched attachments cannot wake an old request.

### Tool approvals

- Channels that support buttons (Telegram / Feishu / Discord / Slack / QQ / LINE / Google Chat) use **native button** approvals; other channels use **text-reply** approvals, except the first WeCom version, which uses the app UI.
- **Auto-approve tools** (an account-level toggle, off by default): once enabled, tool approvals are skipped. If a skipped call would have hit the strictest approval, an audit warning is logged.

### Reply modes (imReplyMode)

The first WeCom version always uses final replies. Other channels offer three modes in the account editor or through `/imreply`:

| Mode | Behavior |
| --- | --- |
| **Per round (split)** (`split`, default) | Each round's narration and media are sent as separate messages in chronological order; on streaming-capable channels each round is a genuine typewriter effect |
| **Final answer only** (`final`) | Sends only the last round's text, with media at the end, and does not enable streaming |
| **Streaming preview** (`preview`) | Streaming-capable channels render one continuously growing merged message; channels that don't support it automatically fall back to "Final answer only" |

The AI's thinking process is discarded by default; `/reason on` turns it on.

With account-level native streaming enabled, Slack and Telegram carry the whole turn in one platform-native stream; `split / final / preview` still decides how rounds are presented. Slack can show task or plan status inside the message, with action buttons appearing in its final state. Telegram DMs use a transient draft for progress, then replace it with a rich final message. If the platform's delivery result is ambiguous, Hope does not resend through the compatibility path, avoiding duplicate replies.

> Microsoft Teams is not built in. An isolated connector/plugin evaluation starts only after at least three active design partners and four consecutive weeks with at least 20 weekly active users, or a concrete enterprise contract.

---

## 10.4 Session takeover and handover

The first WeCom version cannot proactively backfill handovers or mirror desktop turns without an original callback anchor. The following live-delivery behaviors depend on channel capabilities.

Each IM chat (channel + account + chat + thread) is associated with only **one** session at any moment, and conversely each session can be taken over by only one IM chat (bidirectional 1:1).

- When a new chat takes over via `/session <id>` or a desktop handover, the old association is released, and the evicted chat receives a "this session has been taken over by another endpoint" notification (which can be silenced). After takeover, one already-completed round of reply is immediately backfilled.
- **Live streaming mirror**: a conversation you trigger on desktop or web, if that session is associated with an IM chat, is mirrored to the IM side in real time (rendered per the reply mode). The two channels are independent of each other — desktop always uses its own stream and is unaffected by the IM reply mode.
- **Messages sent while busy are not lost**: if you send another ordinary message while the previous IM message is still being answered, Hope acknowledges it as queued and processes messages in send order. The FIFO head is inserted into the current IM turn at a safe tool-completion boundary when possible, with another model reply guaranteed afterward; otherwise it automatically starts the next turn after the current reply. If the running turn came from desktop or web, the IM message waits and then runs as a separate IM turn so it cannot inherit owner tool or Knowledge Space permissions. The queue survives app restarts. After an explicit `/stop` or a Stop from desktop, pending IM messages do not restart themselves; send another ordinary IM message to resume the original order, with the new message appended last.
- Results from completed background jobs, Sub-Agents, and scheduled tasks are also posted back to the associated IM chat, or to the scheduled task's delivery targets.

---

## 10.5 Slash commands in IM

You can also use [slash commands](03-chat-and-sessions.md#39-slash-command-reference) in IM, though some behave differently:

| Command | What it does in IM |
| --- | --- |
| `/sessions` | Opens the session picker |
| `/session [<id>\|exit]` | With no argument, shows session info; `<id>` takes over; `exit` unbinds |
| `/fork` | Copies the current conversation history and moves this chat to the new branch |
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
