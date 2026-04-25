---
name: status-report
description: Use when the user asks for a weekly / monthly status report, project update, individual or team progress summary. Produces a tight, scannable update with what shipped, what's in flight, what's blocked, and metrics.
version: 1.0.0
author: Hope Agent
license: MIT
metadata:
  hermes:
    tags: [office, status, reporting, productivity, communication]
    related_skills: [meeting-notes, email-draft]
---

# Status Report

## When to Use

Trigger phrases: "weekly update", "monthly report", "status report", "project update", "1:1 prep", "周报", "月报", "工作汇报".

Match the cadence implied by the request:
- **Daily / standup**: 3 bullets max, "yesterday / today / blockers"
- **Weekly**: full template below
- **Monthly**: weekly template + "themes" section + metrics over time
- **Quarterly / OKR**: include progress against named goals + risk assessment

This skill optimizes for **the reader's time, not yours**. A status report is read in 60 seconds or skipped.

## The Weekly Template

```markdown
# <Your Name / Team Name> — Week of <YYYY-MM-DD>

## TL;DR
<2 sentences. What's the state of the world? Use plain English.>

## ✅ Shipped This Week
- <bullet — outcome, not activity. Link if relevant.>
- <bullet — outcome.>

## 🔄 In Flight
- <project> — <what stage, expected ship date>
- <project> — <what stage, expected ship date>

## ⚠️ Risks / Blockers
- <risk> — <impact + what's needed to unblock>

## 📊 Metrics (optional)
| Metric | This Week | Last Week | Δ |
|--------|-----------|-----------|---|
| <name> | <value> | <value> | <±> |

## 📅 Next Week
- <top 3 priorities, in order>
```

## Workflow

1. **Gather inputs** via `recall_memory` (if scope=session/project) and `ask_user_question`:
   - Date range covered (default: last 7 days)
   - Audience (manager? cross-functional? company all-hands?) — affects depth and jargon
   - Key projects to cover
   - Whether metrics are required and what the baseline is

2. **Categorize ruthlessly** — every bullet must fit exactly one bucket:
   - **Shipped** = merged / launched / delivered / decided. Past tense.
   - **In Flight** = work in progress, not yet shipped. Has an ETA.
   - **Risks/Blockers** = something that will hurt the plan if not addressed. Has a specific owner ask.

3. **Write outcomes, not activity**:
   - Bad: "Worked on auth migration. Had 3 design meetings."
   - Good: "Auth migration: design approved, infra ready. Cutover scheduled for May 5 (low risk)."

4. **Quantify when possible**:
   - "Shipped feature X" → "Shipped feature X (5% adoption in week 1, target was 3%)"
   - "Faster builds" → "CI builds dropped from 12m → 7m (median)"

5. **Sequence priorities by impact** — the first item in "Next Week" should be the highest-leverage thing. Don't bury it.

6. **Save state** — offer to save the report to memory or to compose an email via `email-draft`:
   ```
   Save this status to memory for next week's diff?
   Send to <manager> as an email?
   ```

## Style Rules

- **TL;DR is mandatory** — readers should know whether to read further in 5 seconds
- **Bullets, not paragraphs** — long sentences are a sign of unclear thinking
- **One bullet = one item** — don't smuggle 3 things into "and also"
- **Past tense for shipped, present for in flight, conditional for risks**
- **Drop hedge words** — "I think we kind of made progress on …" → "We shipped …" or "We're 70% done with …"
- **No vanity metrics** — if it's going up regardless of your effort, it's not your metric
- **Acknowledge what didn't ship** — silence on planned-but-undelivered items damages trust more than honesty

## Cadence-Specific Adjustments

### Daily / Standup (≤3 bullets)
```markdown
- Yesterday: <one outcome>
- Today: <one focus>
- Blockers: <none / specific ask>
```

### Monthly (add to weekly template)
```markdown
## 🎯 Themes
- <pattern across the 4 weeks: what got better, what's recurring>

## 📈 Trends
| Metric | Month-over-Month | Notes |
|--------|------------------|-------|
| ... | ... | ... |

## 🔭 Looking Ahead (next month)
- <strategic priorities, not tactical tasks>
```

### Quarterly / OKR (replace template)
```markdown
# Q<N> Review — <Team>

## Goals Status
| OKR | Target | Actual | Status | Confidence |
|-----|--------|--------|--------|------------|
| <name> | <metric> | <metric> | <On Track / At Risk / Off Track> | <0.0–1.0> |

## What Went Well
- <pattern, not single incident>

## What Didn't
- <honest pattern, with the lesson>

## Q<N+1> Bets
- <top 3 priorities for next quarter, with rationale>
```

## Audience-Specific Adjustments

| Audience | Adjust |
|----------|--------|
| Direct manager | Include risks honestly; manager wants to help unblock |
| Cross-functional team | More context; less internal jargon |
| Company all-hands | Outcomes only; cut planning details |
| Skip-level / exec | TL;DR + 3 bullets total; skip metrics unless they're 1-2 headline numbers |
| External (board, investors) | Headline outcomes + roadmap; never blockers without solutions |

## Common Pitfalls

| Mistake | Fix |
|---|---|
| Listing every Jira ticket | Group into ≤5 outcomes, name them |
| "Working on …" everywhere | Pick a stage: design / implementation / review / shipped |
| Burying bad news | Lead with "TL;DR: <project> slipping by 1 week, here's why" |
| Vague metrics ("better", "faster") | Specific numbers or skip the metric |
| Same priorities every week | Either reframe progress or admit it's stuck |
| No "next week" section | Always have one — shows you're proactive, not just reactive |
| Word-soup TL;DR | Two sentences max. If you can't compress, you don't understand the state. |

## Example

Input: "Write my weekly update — I shipped the auth refactor PR, I'm finishing the dashboard search feature, the embedding migration is blocked on infra, and I want to start the i18n rewrite next week."

Output:
```markdown
# Alice — Week of 2026-04-25

## TL;DR
Shipped auth refactor; dashboard search done by Friday. Embedding migration blocked on infra capacity (need decision by Tuesday).

## ✅ Shipped This Week
- Auth refactor PR merged (#1234) — removes legacy session middleware, paves way for OAuth migration.

## 🔄 In Flight
- Dashboard search — backend + UI complete, integration tests pending. Ship target: Friday.

## ⚠️ Risks / Blockers
- Embedding migration — blocked waiting on infra team's capacity decision. **Need by Tuesday** or May ship date slips.

## 📅 Next Week
1. Land dashboard search to prod.
2. Kick off i18n rewrite (12 locales, scoping doc first).
3. Unblock embedding migration once infra responds.
```
