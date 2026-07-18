# 04 · Memory

The memory system lets the AI **remember you across sessions**—your preferences, your project background, your long-term habits. You only need to understand three things: **always remember** (Core memory), **recall when relevant** (on-demand recall), and **learn from conversations** (automatic / manual consolidation). More advanced profiling and structured organization all live in the advanced area and, by default, need no attention from you.

The end of this chapter also covers a memory-related feature, [behavior awareness](#46-behavior-awareness).

**Main entry**: Settings → **Memory** (six tabs: Overview / Memory Settings / Memory Management / Dreaming / Profile / Claims).

**In this chapter**

- [4.1 Three tiers of memory: Global / Agent / Project](#41-three-tiers-of-memory-global--agent--project)
- [4.2 Core memory and the auto-memory index](#42-core-memory-and-the-auto-memory-index)
- [4.3 Memory recall: bringing back relevant context](#43-memory-recall-bringing-back-relevant-context)
- [4.4 Learning from conversations](#44-learning-from-conversations)
- [4.5 Advanced: offline consolidation, profile, and correction (Dreaming)](#45-advanced-offline-consolidation-profile-and-correction-dreaming)
- [4.6 Behavior awareness](#46-behavior-awareness)

---

## 4.1 Three tiers of memory: Global / Agent / Project

A single memory can belong to one of three tiers, with priority **Project > Agent > Global**:

- **Global**—available to all sessions (for example, "my name is X, and I prefer replies in Chinese").
- **Agent**—only available to sessions that use this Agent (so different assistants don't bleed into each other).
- **Project**—only visible to sessions under that project, and **deliberately will not leak into unrelated sessions** (for example, a project's technical decisions).

**Entry**: Settings → Memory → Memory Management, where you can filter by scope, add, remove, and edit. In a conversation you can also use `/remember` to save, `/forget` to delete, and `/memories` to view.

---

## 4.2 Core memory and the auto-memory index

Each tier (Global / Agent / Project) has one `MEMORY.md` **index** plus several **topic bodies**:

- **Core memory (the MEMORY.md index)** is a short, stable list (roughly 200 lines / 25 KB max). It always enters the AI's context, ensuring that the "always remember" content is reliable and human-editable.
- **Topic bodies** hold the detailed content, read by the AI on demand, so they don't constantly occupy the context.

You can view and edit the Core memory of all three tiers directly in Settings → Memory → Memory Management.

> The AI's automatic learning flow **can only "propose" promoting content into Core memory; it can never silently rewrite the Core memory you maintain**—this is a safety line. Turning off automatic learning also does not delete any existing memory files.

**Related settings**:

| Setting | Default | Effect |
| --- | --- | --- |
| Memory master switch | On | When off, the AI-side Core memory / recall / learning / memory tools are all zeroed out (only your own management UI remains) |
| Core memory switch | On | Stops injecting Core memory, but does not delete the files |
| Core memory budget | 1600 tokens | Cap on the stable injection (choose 1000 / 1600 / 2400 or custom) |

---

## 4.3 Memory recall: bringing back relevant context

"Recall" means finding relevant older memories during a conversation and injecting them. It comes in three forms:

- **Model on-demand recall (always available)**—even with every form of automatic recall turned off, the AI can still call the memory tools itself to look things up, and they return the **full original text**.
- **Fast Recall (must be turned on by you)**—when enabled, every round runs a zero-cost (no model call) retrieval that automatically picks a few of the most relevant memories to inject. **Off by default.**
- **Deep Recall (a secondary switch)**—takes the Fast Recall results and re-ranks and refines them with a model, which is more accurate but adds latency and cost. **Off by default**, and only usable once Fast Recall is enabled.

**Entry**: Settings → Memory → Overview, the "Recall" card.

| Setting | Default | Effect |
| --- | --- | --- |
| Auto-recall relevant memories | **Off** | Whether to run Fast Recall automatically each round |
| Include structured claims | On (only takes effect when recall is on) | Whether Fast Recall includes facts that have been structured |
| Deep Recall | **Off** | Whether to re-rank with a model (more accurate, slower) |

> Turning off automatic recall is **not** the same as turning off memory—the AI still uses Core memory, and it can still query the memory tools on its own.

---

## 4.4 Learning from conversations

The AI can automatically extract facts worth remembering during a conversation. There are three modes (Settings → Memory → Overview, the "Learning" card):

- **Automatic learning (default)**—extracts automatically; ordinary items with a clear scope go straight into the store, while conflicting / sensitive / uncertain-scope ones enter the review queue.
- **Review first**—all automatic memories first enter the review queue and stay invisible to the AI until you approve them.
- **Manual only**—no automatic extraction; only what you explicitly save is written.

Automatic extraction runs in the background without interrupting the chat (triggered after a certain amount of conversation accumulates). The review queue lives in the Memory Management tab and shows the number of items awaiting confirmation.

---

## 4.5 Advanced: offline consolidation, profile, and correction (Dreaming)

Dreaming is an **offline memory-consolidation** capability. When the app is idle, on a schedule, or manually triggered, it organizes scattered memories into an auditable, correctable long-term mind. It is on by default (scheduled triggering off by default); for ordinary users it is "the background quietly getting smarter," and you can also leave it entirely alone.

- **Dream Diary**—each consolidation writes a Markdown narrative recording what was "remembered / solidified" that round, which you can browse in the Dashboard's Dreaming tab.
- **Structured Claims**—upgrades scattered sentences into structured facts with evidence, scope, expiry, and confidence; expired ones are automatically suppressed, and conflicts enter review instead of being overwritten automatically. View and manage them in Settings → Memory → Claims.
- **User Profile**—automatically synthesizes a readable profile summary (your communication style, work habits, and long-term preferences), viewable in Settings → Memory → Profile.
- **Correction loop (Lucid Review)**—**only you can change memories; the AI cannot change them on its own.** You can:
  - **Edit** a claim (change its content / approve / mark as stale / change its scope / pin it);
  - **Forget** a claim (archive it while keeping the audit trail, or delete it permanently).
  - Every correction you make is the "highest authority."

**Settings** (Settings → Memory → Dreaming, medium risk):

| Setting | Default | Effect |
| --- | --- | --- |
| Dreaming master switch | On | Whether to enable offline consolidation |
| Idle trigger | On / 30 minutes | Runs automatically once idle reaches the threshold |
| Scheduled trigger | **Off** / 3 a.m. daily | Scheduled overnight consolidation |
| User profile synthesis | On | Whether to generate a profile summary |

> Dreaming is responsible for "generating / organizing" memories, but whether the conversation actually **uses** these organized results is still governed by [the recall switches in 4.3](#43-memory-recall-bringing-back-relevant-context).

> **Semantic retrieval**: retrieving memories by meaning relies on an embedding (vector) model; see [02 · Memory embedding model](02-models-and-providers.md#210-memory-embedding-model).
>
> **Incognito sessions**: an [Incognito session](03-chat-and-sessions.md#36-incognito-sessions) is completely "blind" to the memory system—nothing is injected, extracted, or stored.

---

## 4.6 Behavior awareness

Behavior awareness lets the AI in each session know "what you are doing in other parallel sessions at the same time," so it can understand cross-session references like "that bug from earlier" or "the CI I'm debugging in the other window."

- **Off by default** (the overall master switch is off by default). Once enabled, there are two modes: `Structured` (zero cost, aggregates a list of other sessions from the database) and `LLM summary` (additionally uses a model to generate a natural-language summary, which has a cost).
- **Entry**: Settings → Chat & Context → Behavior awareness; each session can also override it individually via the **eye icon** next to the temperature slider in the input box; in a conversation you can use the `/awareness` command.
- By default it excludes Scheduled Tasks, IM, and sub-Agent sessions; Incognito sessions never take part.

| Setting | Default | Effect |
| --- | --- | --- |
| Master switch | **Off** | Whether to enable behavior awareness |
| Mode | Structured | Off / Structured / LLM summary |
| Max related sessions | 6 | How many other sessions to mention at most |
| Look-back window | 72 hours | How far back to look for sessions |

---

## Next steps

- Manage notes and materials → [05 · Knowledge Space](05-knowledge-space.md)
- Configure vector retrieval for memory → [02 · Memory embedding model](02-models-and-providers.md#210-memory-embedding-model)
