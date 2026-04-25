---
name: email-draft
description: Use when the user asks to draft, polish, translate, or reply to an email. Produces a clean draft with subject line, greeting, body, and sign-off, plus a pre-send self-check.
version: 1.0.0
author: Hope Agent
license: MIT
metadata:
  hermes:
    tags: [office, email, communication, writing, productivity]
    related_skills: [meeting-notes, status-report]
---

# Email Draft

## When to Use

Trigger phrases: "draft an email", "reply to this email", "polish this email", "翻译这封邮件", "write an email to <X>".

Don't trigger for: chat messages, Slack DMs, in-app notifications, marketing copy. Those are different formats.

## Output Format

Always produce the draft in this layout. Use a fenced code block so the user can copy it cleanly:

```
Subject: <concise, action-oriented, ≤60 chars>

Hi <Name>,

<opening line — context or thanks>

<body — one focused paragraph per point, max 3 paragraphs>

<call to action / next step / explicit ask>

Best,
<Sender>
```

If the language isn't English, adapt greeting and sign-off to the local convention:
- 中文: `<Name>，你好` / `祝好，<Sender>`
- 日本語: `<Name>様` / `よろしくお願いいたします。<Sender>`
- Français: `Bonjour <Name>,` / `Cordialement, <Sender>`
- 한국어: `<Name>님께` / `감사합니다, <Sender>`

## Workflow

1. **Clarify minimal essentials** via `ask_user_question` if missing:
   - Recipient and rough relationship (peer / manager / external client / cold outreach)
   - One-sentence purpose ("schedule a meeting" / "decline a vendor" / "ask for clarification")
   - Tone (formal / collegial / brief)
   - Preferred language
   - Any constraints to mention (deadline, budget, link to attachment)

2. **Subject line first** — the subject is what gets read. Make it specific and action-oriented:
   - Bad: "Quick question"
   - Good: "Q3 budget approval — need sign-off by Friday"
   - Bad: "Following up"
   - Good: "Followup on June 12 design review action items"

3. **Body** — three paragraph max:
   - **Paragraph 1**: context / why you're writing
   - **Paragraph 2**: the specific request, decision, or information
   - **Paragraph 3**: clear next step (what you want them to do, by when)

4. **Pre-Send Self-Check** — after producing the draft, run through this checklist out loud (in your reply to the user, before the draft):

   - [ ] Subject is specific and action-oriented (not "Hi" / "Question")
   - [ ] Recipient name is correct (no "Hi <Name>" placeholder left)
   - [ ] Purpose stated in the first 2 sentences
   - [ ] Concrete ask with deadline, not vague
   - [ ] No internal jargon if recipient is external
   - [ ] Tone matches relationship (formal vs collegial)
   - [ ] No accidental "Reply All" implications (if forwarding, mention)
   - [ ] No PII / secrets that shouldn't be in email
   - [ ] Attachments mentioned in body if any
   - [ ] Sign-off matches the user's actual name

   If any box fails, revise the draft before showing it.

## Common Patterns

### Cold outreach (asking for time)

```
Subject: 15 min on <topic>?

Hi <Name>,

I'm <role> at <company>. We're working on <one-sentence context>, and I noticed your work on <specific thing>.

Would you have 15 minutes in the next two weeks for a quick call? I'm happy to share <X> in return / send a written question if that's easier.

Either way, thanks for considering.

Best,
<Sender>
```

### Decline (politely)

```
Subject: Re: <original subject>

Hi <Name>,

Thanks for thinking of me / sending this.

I've decided to pass for now because <one-sentence reason>. Specifically, <constraint>.

Happy to revisit if <condition>.

Best,
<Sender>
```

### Status escalation (to manager)

```
Subject: <Project> blocked on <decision>

Hi <Name>,

Quick heads-up: <project> is blocked on <specific decision> as of <when>.

Context: <2 sentences max — what's happening and what's at stake>.

I'd recommend <option>. Can you confirm by <date>, or shall we discuss live? Open to your input.

Thanks,
<Sender>
```

### Reply with action items

```
Subject: Re: <original subject>

Hi <Name>,

Thanks for the call/email. To make sure we're aligned:

- <decision 1>
- <decision 2>

Action items on my side:
1. <action> — by <date>
2. <action> — by <date>

Action items on your side:
1. <action> — by <date>

Let me know if I missed anything.

Best,
<Sender>
```

## Style Rules

- **Front-load the ask** — most readers skim. The first sentence after greeting should set context; the second should hint at the action.
- **Don't apologize unless needed** — drop "Sorry to bother you" / "I hope this isn't a stupid question". Just ask.
- **One ask per email** — multiple requests dilute response rate. Split into separate emails or use a numbered list.
- **No "per my last email"** — replace with "to recap" or restate the question.
- **Keep paragraphs short** — 2-4 lines max. Wall-of-text emails get archived unread.
- **Active voice** — "I'll send the draft Monday" beats "the draft will be sent Monday".

## Translation Mode

When the user asks to translate an email:
1. Preserve the structure (subject / greeting / body / sign-off)
2. Adapt greeting and sign-off to target language conventions (see top of skill)
3. Note any culturally-tricky phrasings — flag with a `[note]` comment outside the draft
4. If the source has tonal markers (formal Japanese keigo, polite Korean, etc.), preserve register

## Common Pitfalls

| Mistake | Fix |
|---|---|
| Generic subject ("Hi", "Followup") | Specific subject naming the topic + action |
| Burying the ask in paragraph 4 | Move to paragraph 1 or 2 |
| "Hope you're well" filler when familiar | Skip — go straight to topic |
| Vague deadline ("when you can") | Specific date or "no rush, just FYI" |
| Mixing 3 unrelated asks | Split into 3 emails |
| Auto-translation kept English greeting | Localize properly (`你好`, `안녕하세요`) |
