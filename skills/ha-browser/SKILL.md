---
name: ha-browser
description: "Hope Agent browser automation — the standard `status → tabs → snapshot → act` loop, stale-ref recovery rules, and what to do when login / 2FA / captcha / camera-prompt / dialog blocks progress. Load this skill whenever you reach for the `browser` tool. Trigger on: user asks the agent to open / control / click / scrape / log into / verify something in a web app ('open X and click Y', '打开 X 然后点击 Y', 'log into my Gmail', 'scrape this page', 'fill out the form on X'); user reports a flow that requires real browser context (cookies, JS-rendered content, OAuth)."
version: 1.0.0
author: Hope Agent
license: MIT
allowed-tools: [browser, ask_user_question, read, job_status]
status: active
---

# Hope Agent Browser — operating loop

The `browser` tool exposes 8 high-level actions. Default backend is Hope Agent's Chrome Extension + Native Messaging Host, which can control the user's real Chrome tabs after they install the extension and native host. If the extension is unavailable, generic browsing can fall back to the managed/user_attach CDP backend, but real Chrome tab/session tasks must fail closed and ask the user to install or enable the extension.

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

When the user explicitly asks for their current Chrome, an already-open tab, their logged-in session, or browser extensions/cookies from their daily Chrome, use `tabs.open_user_tabs` and `tabs.claim` first. Do not launch a managed CDP profile and pretend it is the user's Chrome.

Real Chrome access uses the normal Hope Agent tool approval flow. `tabs.open_user_tabs`, `tabs.claim`, extension numeric-id `tabs.select`, `observe.kind=downloads`, `control.download_cancel`, and `control.raw_cdp` may ask unless the session policy, AllowAlways, Smart mode, or YOLO allows them.

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

## Real Chrome tabs

Use the extension-backed tab lease protocol whenever the task depends on the user's real Chrome state:

```
status
tabs.open_user_tabs
tabs.claim target_id="<chrome-tab-id>"
snapshot / act / observe
tabs.finalize
```

Rules:

- `tabs.claim` takes temporary control of a real user tab. Release it with `tabs.release` or `tabs.finalize` when the task ends.
- `tabs.select` with a numeric extension tab id also activates and controls that real Chrome tab. Prefer `tabs.claim` when your intent is explicit takeover; use `tabs.select` for tab switching after you know the target id.
- `tabs.finalize` for a claimed user tab must not close the tab by default; it releases Hope Agent control.
- `tabs.new` creates a Hope-controlled automation tab. `tabs.finalize` closes agent-created tabs unless their target id is listed in `keep`.
- If a tab is already claimed by another Hope session, do not steal it unless the user explicitly asked to take over; then pass `steal:true`.
- If the extension is missing or disabled, real Chrome tasks are blocked. Tell the user to open Settings -> Browser and install/enable the Chrome Extension + Native Host. Generic browsing may continue with CDP fallback, but that is an isolated Hope Agent browser, not the user's current Chrome.

## When NOT to use `browser`

Browser automation is the most expensive tool you have — every step is round-trips, every snapshot is a 30KB blob, every screenshot is a hundred KBs. Don't reach for it when something cheaper works:

- **Public web content** → `web_fetch` (HTML/Markdown) or `web_search` first. Open a browser only if the page is JS-rendered, gated by cookies, or you genuinely need to interact (click, fill).
- **One-shot file download** → `web_fetch` saves bytes; `browser.snapshot pdf` is only for the rendered DOM-as-PDF case.
- **API call** → if the site has an API and you can hit it with `exec curl` or `web_fetch`, do that — no browser needed.

## Common pitfalls

- **Two snapshots in a row without anything in between**: you wasted a turn. Snapshot once, then `act` a few times, then re-snapshot.
- **`act.kind=fill` with a ref that points to a `<div>` instead of `<input>`**: the snapshot output annotates `role`; check it before filling. Use `evaluate` to inspect the DOM if unsure.
- **Trusting the URL on a redirect-heavy site**: after `navigate.go`, the page may bounce through several URLs. Re-snapshot after navigation, do not assume the URL in your `navigate.go` argument matches the current page.
- **Forgetting `observe`**: when something silently fails, `observe.kind=console` and `observe.kind=page_errors` often surface the cause for free. In the extension backend those streams are filtered to the active controlled tab. `observe.kind=downloads` reads Chrome download activity and follows normal approval policy.

## Common CDP error strings

- `"Cannot find context with specified id"` / `"detached"` / `"no such element"` — stale ref or page replaced. Recovery: re-snapshot.
- `"selected page closed"` — active tab was closed externally. Recovery: `tabs.list` → `tabs.select` on a live tab.

## Choosing `profile.op=launch profile=`

```
profile=managed       → automation, scrapers, anything that should NOT inherit
                        the user's login state. Ephemeral runner under
                        ~/.hope-agent/browser/managed-runner/, OS-picked
                        debug port. This is the default — omit `profile=`
                        to get it.

profile=user_attach   → routine work where you DO want a persistent Hope Agent
                        CDP profile (sign in once, keep the cookies, reuse
                        extensions inside that Hope Agent browser).
                        Lives at ~/.hope-agent/browser/user-attach/, pinned
                        to port 9222. This is the recommended way to maintain
                        "the user's hope-agent browser" — populate the logins
                        once and reuse them. No extra approval — same risk
                        profile as managed.

profile=<other>       → user-defined profiles in AppConfig.browser.profiles
                        (per-profile user-data-dir / port / executable /
                        headless / extra args). Treat them like user_attach
                        in terms of persistence assumption.
```

The legacy `target=managed|user_attach` parameter is gone — only `profile=<name>`
is accepted now for CDP lifecycle operations. A previous `target=system` option
was removed; do not try to attach CDP to the user's default Chrome profile. When
the user wants their real daily Chrome with current logins, use the extension
path (`tabs.open_user_tabs` → `tabs.claim`) or ask them to install/enable the
extension if it is missing.

## When Chrome / Chromium is missing entirely

If `profile.op=launch` fails with "No Chrome / Chromium found...", suggest
ONE of:

1. Install Google Chrome system-wide (best user experience).
2. Run `browser(action="profile", op="install_runtime")` — downloads a
   pinned Chromium snapshot (~150 MB) into `~/.hope-agent/browser/runtime/`.
   This is an async job; poll `job_status` for progress, or check the
   settings → Browser doctor banner.
3. Pass `executable_path` explicitly if the user has a custom Chrome build.

## Quick reference — full action surface

```
browser(action="status")
browser(action="profile", op="list")
browser(action="profile", op="launch", profile="managed", headless=false)
browser(action="profile", op="launch", profile="user_attach")
browser(action="profile", op="install_runtime")             # ← async, downloads Chromium
browser(action="profile", op="connect", url="http://127.0.0.1:9222")
browser(action="profile", op="disconnect")
browser(action="tabs", op="list")
browser(action="tabs", op="new", url="https://...")
browser(action="tabs", op="select", target_id="<id>")
browser(action="tabs", op="close", target_id="<id>")
browser(action="tabs", op="open_user_tabs")      # extension-only, real Chrome
browser(action="tabs", op="claim", target_id="<id>", steal=false)
browser(action="tabs", op="release", target_id="<id>")
browser(action="tabs", op="finalize")
browser(action="tabs", op="finalize", keep=["<agent-tab-id>"]) # keep Hope-created tabs open
browser(action="navigate", op="go", url="https://...")
browser(action="navigate", op="back" | "forward" | "reload")
browser(action="snapshot", format="role")
browser(action="snapshot", format="screenshot", image_format="jpeg", full_page=false)
browser(action="snapshot", format="screenshot", annotate=true)
browser(action="snapshot", format="pdf", paper_format="a4")
browser(action="act", kind="click", ref=N)
browser(action="act", kind="dblclick", ref=N)
browser(action="act", kind="fill", ref=N, text="...")    # clears the field then sets value
browser(action="act", kind="hover", ref=N)
browser(action="act", kind="drag", ref=N, target_ref=M)
browser(action="act", kind="select", ref=N, values=["..."])
browser(action="act", kind="press", key="Enter")          # one keypress on the focused element
browser(action="act", kind="upload", ref=N, file_path="/path/to/file")
browser(action="observe", kind="console" | "network" | "page_errors" | "downloads", since=<unix-millis>)
browser(action="control", op="resize", width=1280, height=720)
browser(action="control", op="scroll", direction="down", amount=500)
browser(action="control", op="wait_for", text="...", timeout=30000)
browser(action="control", op="handle_dialog", accept=true, dialog_text="...")
browser(action="control", op="evaluate", expression="document.title")
browser(action="control", op="download_cancel", download_id=7) # normal tool approval
browser(action="control", op="raw_cdp", method="Accessibility.getFullAXTree", params={}) # normal tool approval; advanced path, params are not payload-scanned
```
