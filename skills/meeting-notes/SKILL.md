---
name: meeting-notes
description: Use when the user asks to capture, structure, or summarize meeting notes / call notes / 1:1 discussion / standup notes. Produces a standard template with attendees, agenda, decisions, action items (owner + deadline), and open questions.
version: 1.0.0
author: Hope Agent
license: MIT
metadata:
  hermes:
    tags: [office, meeting, productivity, notes, action-items]
    related_skills: [status-report, email-draft]
---

# Meeting Notes

## When to Use

Trigger phrases: "take meeting notes", "summarize this meeting", "1:1 notes", "standup notes", "post-mortem notes", "会议纪要", "记录会议".

Also trigger when the user pastes raw transcript / bullet points and asks for cleanup.

## The Template

Always produce notes with these five sections, in this order. Skip a section only if the user explicitly says it doesn't apply.

```markdown
# <Meeting Title> — <YYYY-MM-DD>

**Attendees:** <name1>, <name2>, ...
**Duration:** <Nm>
**Recording:** <link or "n/a">

## Agenda

1. <topic>
2. <topic>

## Discussion

### <Topic 1>
- <key point>
- <key point with context>

### <Topic 2>
...

## Decisions

- ✅ <decision 1> — rationale: <why>
- ✅ <decision 2> — rationale: <why>

## Action Items

| # | Action | Owner | Deadline | Status |
|---|--------|-------|----------|--------|
| 1 | <action verb + outcome> | <person> | <YYYY-MM-DD> | <Open/Done/Blocked> |
| 2 | ... | ... | ... | ... |

## Open Questions

- <question that didn't get an answer>
- <question that needs follow-up>
```

## Workflow

1. **Gather input** — if the user only sketched bullet points, ask for missing essentials via `ask_user_question`:
   - Meeting title and date (default to today if not given)
   - Attendees (at least names)
   - Whether decisions / action items already exist or you should infer them

2. **Extract** — read the transcript / bullets carefully. For each line, classify:
   - Background context → Discussion
   - "We agreed", "We'll go with", "Decided to" → Decisions
   - "<X> will <verb>", "Action: ...", "Owner: ..." → Action Items
   - "?", "TBD", "needs follow-up" → Open Questions

3. **Action Item Discipline (the high-value part)**:
   - Every action MUST have an owner (a real name, not "team")
   - Every action MUST have a deadline (a specific date, not "soon" or "next week")
   - If owner or deadline is missing, surface it explicitly with ⚠️ before the row, e.g.: `⚠️ Owner not assigned: <action>` — let the user resolve
   - Action verb first ("Draft …", "Review …", "Ship …"), not vague ("Look into …")

4. **Decision Discipline**: each decision should be one sentence with a brief "why" so future readers can reconstruct context.

5. **Save (optional)** — if the user wants to remember decisions / action items for future sessions, offer to save key items via `save_memory` (scope: "session" or "project"). Don't auto-save without confirming.

## Style Rules

- Past tense for Discussion ("we discussed …"), present tense for Decisions ("we go with …"), imperative for Action Items ("Draft proposal …")
- Don't editorialize — just record what was said, decided, or assigned
- One bullet = one fact. No multi-clause sentences with embedded sub-points
- Keep technical jargon if it's the team's vocabulary; don't over-explain
- For multi-language meetings, default to the user's preferred language; mark code-switches with `[en]` / `[zh]` if helpful

## Common Pitfalls

| Mistake | Fix |
|---|---|
| Vague action: "Look into auth" | "Draft auth migration plan with timing estimates" |
| No owner: "Team will review" | Surface as `⚠️` and ask who owns it |
| No deadline: "By next week" | Pin to a specific date (`YYYY-MM-DD`); ask if unclear |
| Mixing decisions with discussion | Decisions get the ✅ section; everything else stays in Discussion |
| Listing everything said | Compress — one bullet per substantive point, not per sentence |

## Example

Input:
```
hi all - q3 planning. we need to ship feature X by oct. alice will write the spec
- kevin: should we cut feature Y? yes everyone agrees
- bob will run user research starting next mon
- still unclear who owns infra migration
```

Output:
```markdown
# Q3 Planning — 2026-04-25

**Attendees:** Alice, Kevin, Bob
**Duration:** 30m
**Recording:** n/a

## Agenda
1. Feature X timeline
2. Feature Y status
3. Open ownership questions

## Discussion
### Feature X
- Targeting October ship date

### Feature Y
- Considered cutting from Q3 to focus on X

## Decisions
- ✅ Ship Feature X by October — rationale: aligned with Q3 OKRs
- ✅ Cut Feature Y from Q3 — rationale: capacity constrained

## Action Items
| # | Action | Owner | Deadline | Status |
|---|--------|-------|----------|--------|
| 1 | Draft Feature X spec | Alice | 2026-05-02 | Open |
| 2 | Run user research wave | Bob | 2026-04-28 | Open |

## Open Questions
- ⚠️ Owner not assigned: Infrastructure migration
```
