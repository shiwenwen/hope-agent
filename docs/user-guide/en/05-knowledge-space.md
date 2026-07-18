# 05 · Knowledge Space

The Knowledge Space is your **second brain**—a local-first, AI-native personal knowledge management space. The core principles are simple:

- **A note is a real `.md` file**, the single source of truth, never locked; you can open the same folder in Obsidian / Logseq;
- **AI is a first-class citizen**, able to read, write, and organize notes alongside you;
- **Access is denied by default**; a knowledge space must be explicitly mounted before a session can read or write it.

It differs from the [Memory](04-memory.md) system: memory is one-sentence facts, automatic in the background, low-visibility; the Knowledge Space is full documents, driven by you, maximum-visibility.

**Entry point**: the "Knowledge Space" item in the sidebar (in the top-level navigation).

**In this chapter**

- [5.1 Creating a knowledge space](#51-creating-a-knowledge-space)
- [5.2 Binding an existing Obsidian vault](#52-binding-an-existing-obsidian-vault)
- [5.3 Reading and writing notes with AI](#53-reading-and-writing-notes-with-ai)
- [5.4 Search, backlinks, and the graph](#54-search-backlinks-and-the-graph)
- [5.5 The sidebar AI chat panel](#55-the-sidebar-ai-chat-panel)
- [5.6 Related-note hints and the inspiration Sprite](#56-related-note-hints-and-the-inspiration-sprite)
- [5.7 AI autonomous maintenance](#57-ai-autonomous-maintenance)
- [5.8 Access control](#58-access-control)

---

## 5.1 Creating a knowledge space

Creating a knowledge space is essentially creating a folder that holds `.md` notes. On first install, an internal space and a welcome note are set up automatically.

The note editor is built on CodeMirror and has 5 modes: `Source / Preview / Split / Live (WYSIWYG, still plain Markdown underneath) / Outline (read-only)`.

> Indexes such as the vectors and the graph are all "rebuildable caches"—even if deleted, they can be fully rebuilt from the `.md` files, so your content is always safe.

---

## 5.2 Binding an existing Obsidian vault

You can directly "light up" an existing Obsidian / Logseq note vault and treat it as a knowledge space. External changes to files are **synced in real time** (a full reconciliation also runs on binding and at startup).

> **An external vault is fully read-only by default**—neither the AI nor the interface can write to it. You must manually enable "Allow writing to the external vault" in the space settings to unlock editing. **Background autonomous maintenance never writes to an external vault**, regardless of this switch. All writes are atomic and conflict-protected; when the interface encounters an external change, it shows an "external modification conflict" prompt (Reload / Keep my changes).

---

## 5.3 Reading and writing notes with AI

The AI operates on notes directly through the `note_*` family of tools: create, read, modify, append, delete, search, backlink, find backreferences, look up by tag, generate a graph, rename and move (automatically rewriting the backlinks that point to it), split a long document into atomic notes, generate a topic index (MOC), turn a conversation into a note, and more.

Writing `[[note name]]` in a message injects that note's content into the conversation (as "untrusted external data"—it is never treated as an instruction).

---

## 5.4 Search, backlinks, and the graph

- **Search**: combines full-text search and vector (semantic) search, then returns the most relevant notes after fused ranking. There is also a "merged search" tool that queries both the memory store and the note store at once (the two result sets are ranked independently and not interleaved).
- **Atomic notes**: you can split a long document into 2–8 atomic notes, or generate a MOC index page by topic / tag.
- **Graph**: a backlink relationship graph view; you can drag to pin the layout and save it.

> The Knowledge Space's semantic search uses an **embedding model independent of memory** (the two are physically isolated). Without an embedding model configured, it simply falls back to keyword search.

---

## 5.5 The sidebar AI chat panel

In the right-hand column of the Knowledge Space you can chat with the AI directly, combining the note you currently have open to do cross-note search and writing—without switching back to the main chat.

- A single note can have multiple conversations; opening a note loads the most recent one by default, and you can switch / search history.
- You can "distill" a conversation's answer into a note (create / update the current one / append; it is written to disk only after you review the diff).
- Selecting a passage of text lets you "add to conversation" for the AI to rewrite it, or use the floating "quick rewrite" bar to rewrite in one shot (applied after you preview the diff).

> These conversations are hidden sessions; they do not appear in the main session list or global search, and they are unavailable in incognito sessions.

---

## 5.6 Related-note hints and the inspiration Sprite

### Related-note hints (passive recall)

**On by default**. On each conversation turn, the system finds notes related to the current topic in the background (no model call, very cheap) and appends a short list of "related note" titles to the prompt.

| Setting | Default | Purpose |
| --- | --- | --- |
| Related-note hints | On | Whether to automatically append related-note titles |
| Max count | 5 | How many to list |
| Show summaries | Off | Whether to add a one-line summary under each title (uses more tokens) |

> Hints appear only when the current session has been granted access to a knowledge space; incognito sessions get zero hints, and IM channels get zero access when unauthorized.

### The inspiration Sprite

A proactive writing-companion assistant—when you pause while editing a note, it may pop a transient bubble into the chat panel with a writing suggestion, feedback, or a related-note hint. **Off by default**.

**Entry point**: the ✨ button at the top of the chat panel, or Settings → Knowledge → Sprite / inspiration mode. It has adjustable trigger conditions (how long an editing pause must last, how many characters you must write before it reacts), a cooldown, and a per-hour cap. **The Sprite never appears in incognito sessions**.

---

## 5.7 AI autonomous maintenance

The Knowledge Space can let the AI **automatically organize** your internal knowledge base in the background—auto-linking, rescuing orphaned notes, filling in metadata, deduplicating and merging, adding tags, maintaining index pages, and more—producing a batch of "maintenance proposals" for you to review.

**All off by default**. Even once enabled, you must confirm in the maintenance panel before the AI touches your notes (unless you additionally turn on "auto-apply").

**Settings** (Settings → Knowledge → Autonomous maintenance, **high risk**):

| Setting | Default | Purpose |
| --- | --- | --- |
| Enable background maintenance | **Off** | Whether to scan on schedule and produce proposals |
| Idle trigger / scheduled trigger | Off | When to scan |
| Auto-apply | Off | Whether to skip review and write notes directly (enable with caution) |

> This item is **high risk**, because "auto-apply" means letting the AI autonomously rewrite your knowledge base. In addition, the Knowledge Space has a separate "**material bay**" where you can import material such as text / PDF / Word / audio-video transcripts / image OCR / URLs; when compiling material into notes, it first shows you a diff to review. Related settings such as material size limits and search ranking parameters all live in the Knowledge Space settings.

---

## 5.8 Access control

Unlike memory, the Knowledge Space is not globally visible—different vaults are isolated from one another, and **access is denied by default and requires an explicit mount**:

- **Denied by default + explicit mount**: to let a session or project read or write a knowledge space, you must explicitly mount it in the knowledge selector of the input box or in the project settings (with optional read / write permission). Without a mount, the AI cannot even see the note tools.
- **Zero access in incognito sessions**—no knowledge space can be accessed at all.
- **IM must be enabled**—IM channels have zero access by default; to open it up, you must enable knowledge-space access for that account in the desktop settings, and in group chats also confirm each one with `/kb on`.
- **Your own admin interface is the exception**—the desktop / web owner interface can see all of your knowledge spaces, no mount required.

---

## Next steps

- From idea to design artifact → [06 · Design Space](06-design-space.md)
- Configure the Knowledge Space's vector search → [02 · Memory embedding model](02-models-and-providers.md#210-memory-embedding-model)
