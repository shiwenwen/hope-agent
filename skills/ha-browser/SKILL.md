---
name: ha-browser
description: "Hope Agent browser automation — the standard `status → tabs → snapshot → act` loop, stale-ref recovery rules, and what to do when login / 2FA / captcha / camera-prompt / dialog blocks progress. Load this skill whenever you reach for the `browser` tool. Trigger on: user asks the agent to open / control / click / scrape / log into / verify something in a web app ('open X and click Y', '打开 X 然后点击 Y', 'log into my Gmail', 'scrape this page', 'fill out the form on X'); user reports a flow that requires real browser context (cookies, JS-rendered content, OAuth)."
version: 1.0.0
author: Hope Agent
license: MIT
allowed-tools: [browser, ask_user_question, read]
status: active
---

# Hope Agent Browser — operating loop

The `browser` tool exposes 8 high-level actions over a single Chrome session. Two backends sit underneath transparently: `chrome-devtools-mcp` (default when Node.js ≥ 18 is on PATH) and direct CDP via `chromiumoxide`. **You do not need to know which backend is active** — refs, ops, and error messages are unified. The desktop user can see which one is running via the BrowserPanel badge.

## The standard loop

Run these in order; never skip a step. Browsers are stateful — assumptions get punished.

```
1. browser(action="status")            # never operate blindly
2. browser(action="tabs",  op="list")  # know what's open before opening more
3. browser(action="tabs",  op="new", url=...)  # only if you actually need a fresh tab
4. browser(action="snapshot", format="role")    # get fresh refs
5. browser(action="act", kind=..., ref=..., ...)
6. when in doubt → re-snapshot
```

A typical "fill the login form" flow is:

```
status → tabs.list → (already on the right tab? if not, tabs.select / tabs.new)
       → navigate.go url="https://app.example.com/login"
       → snapshot format=role           # capture refs
       → act kind=fill   ref=<email>   text="me@..."
       → act kind=fill   ref=<password> text="..."
       → act kind=click  ref=<submit>
       → snapshot format=role           # re-snapshot after navigation
       → verify expected element exists
```

## Refs are tied to the snapshot

`ref` is **only** valid against the most recent `snapshot.role` for the active tab. The moment the page navigates, the DOM mutates (SPA route change, modal opens/closes), or you switch tab — refs are stale.

**Re-snapshot when:**

- you just called `navigate.go / .back / .forward / .reload`
- you switched tab (`tabs.select`)
- an `act` returned an error that looks like "ref not found" / "no such element" / "detached"
- the URL bar in the snapshot output differs from what you expect
- a previous `act` triggered an obvious UI change (modal, page transition, form expansion)

## Stale-ref auto-recovery — what to expect

The tool tries **one** automatic recovery before bubbling up a stale-ref error: it re-snapshots, looks for an element with the same `role` and `text` (or a substring match) as the original ref, and retries with the new ref. On success the result string ends with `(ref auto-recovered)` so you know it kicked in. On failure you get the original error.

Practical rules:

- If a recovery happened, **verify the next action's prerequisites freshly** — a recovered ref means the DOM rearranged.
- If recovery fails, **resnapshot manually and re-plan** — don't keep hammering the same `act` call.
- Recovery only kicks in for `act.kind`. `navigate`, `tabs.*`, and `control.*` do not retry.

## When to stop and ask the user

These five situations are **blocking** — do not guess your way through them. Call `ask_user_question` and wait.

| Signal in the snapshot or error | What you must do |
| --- | --- |
| Login form / email + password / "Sign in" button | Ask the user to sign in, or supply credentials via the right channel. Never type credentials you guessed. |
| 2FA / OTP code prompt | Ask the user for the code (they have the device). |
| CAPTCHA / "I'm not a robot" | Ask the user to solve it. Do not attempt to bypass. |
| Camera / microphone / notification permission prompt | Ask the user — that's a system dialog only they can answer. |
| Browser-native file picker or download confirmation | If you triggered it, `control.handle_dialog`. If it's a host-driven save dialog, ask the user. |

When you have to stop, the right call is roughly:

```
ask_user_question({
  reason: "Browser flow requires you",
  questions: [{ q: "I see a CAPTCHA on https://...; please solve it, then say 'continue'.", ... }]
})
```

## Tab discipline

Multi-tab work loses refs more than anything else. Two rules keep you sane:

1. **Name your tabs as soon as you open them.** Right after `tabs.new`, jot the `target_id` and the URL/role in your reasoning ("tab A11C = github, tab B22D = jira"). Always pass `target_id` explicitly to `tabs.select` instead of relying on "active".
2. **One snapshot per action burst per tab.** Don't snapshot tab A, switch to tab B for two ops, switch back to A, and reuse the old A refs. Re-snapshot when you come back.

## When NOT to use `browser`

Browser automation is the most expensive tool you have — every step is round-trips, every snapshot is a 30KB blob, every screenshot is a hundred KBs. Don't reach for it when something cheaper works:

- **Public web content** → `web_fetch` (HTML/Markdown) or `web_search` first. Open a browser only if the page is JS-rendered, gated by cookies, or you genuinely need to interact (click, fill).
- **One-shot file download** → `web_fetch` saves bytes; `browser.snapshot pdf` is only for the rendered DOM-as-PDF case.
- **API call** → if the site has an API and you can hit it with `exec curl` or `web_fetch`, do that — no browser needed.

## Common pitfalls

- **Two snapshots in a row without anything in between**: you wasted a turn. Snapshot once, then `act` a few times, then re-snapshot.
- **`act.kind=fill` with a ref that points to a `<div>` instead of `<input>`**: the snapshot output annotates `role`; check it before filling. Use `evaluate` to inspect the DOM if unsure.
- **Trusting the URL on a redirect-heavy site**: after `navigate.go`, the page may bounce through several URLs. Re-snapshot after navigation, do not assume the URL in your `navigate.go` argument matches the current page.
- **Forgetting `observe`**: when something silently fails, `observe.kind=console` and `observe.kind=page_errors` often surface the cause for free. (Network observe lags by ~one user request on the MCP backend — see status output for the CDP side-channel readiness note.)

## Backend-specific notes

You should not have to care, but a few error strings differ:

- **MCP backend**: errors from `chrome-devtools-mcp` may say "selected page closed" — that means the active tab was closed externally. Recovery: `tabs.list` to find a live tab, then `tabs.select`.
- **CDP backend**: errors usually carry the underlying CDP message (`"Cannot find context with specified id"` for stale targets). Recovery: re-snapshot.

If you need to confirm the backend, look at the BrowserPanel badge in the desktop UI, or call `status` (the `backend` field).

## Quick reference — full action surface

```
browser(action="status")
browser(action="profile", op="list")
browser(action="profile", op="launch", profile="default", headless=false)
browser(action="profile", op="connect", url="http://127.0.0.1:9222")
browser(action="profile", op="disconnect")
browser(action="tabs", op="list")
browser(action="tabs", op="new", url="https://...")
browser(action="tabs", op="select", target_id="<id>")
browser(action="tabs", op="close", target_id="<id>")
browser(action="navigate", op="go", url="https://...")
browser(action="navigate", op="back" | "forward" | "reload")
browser(action="snapshot", format="role")
browser(action="snapshot", format="screenshot", image_format="jpeg", full_page=false)
browser(action="snapshot", format="pdf", paper_format="a4")
browser(action="act", kind="click", ref=N)
browser(action="act", kind="type" | "fill", ref=N, text="...")
browser(action="act", kind="hover", ref=N)
browser(action="act", kind="drag", ref=N, target_ref=M)
browser(action="act", kind="select", ref=N, values=["..."])
browser(action="act", kind="press", key="Enter")
browser(action="act", kind="upload", ref=N, file_path="/path/to/file")
browser(action="observe", kind="console" | "network" | "page_errors", since=<unix-millis>)
browser(action="control", op="resize", width=1280, height=720)
browser(action="control", op="scroll", direction="down", amount=500)
browser(action="control", op="wait_for", text="...", timeout=30000)
browser(action="control", op="handle_dialog", accept=true, dialog_text="...")
browser(action="control", op="evaluate", expression="document.title")
```
