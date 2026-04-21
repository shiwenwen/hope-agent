---
name: ha-find-skills
description: "Discover and install third-party skills from external registries when the user needs a capability that no currently-active skill covers. Trigger when: (1) the user explicitly asks 'find a skill for X', 'is there a skill that does X', 'install a skill to X', (2) the user requests a well-known integration (Slack, Notion, Trello, GitHub, Hue, Sonos, iMessage, weather, TTS, transcription …) that isn't in the active skill catalog, (3) you are about to hand-write ad-hoc shell / API code for a domain that almost certainly has a published skill. Do NOT trigger if an active skill already covers the need — scan the visible skill catalog first."
always: true
---

# Find Skills

Hope Agent can load skills from multiple sources. When the user needs a capability the active catalog doesn't cover, this workflow finds a candidate on a public registry, confirms with the user, and installs it into `~/.hope-agent/skills/` so it's picked up on next prompt rebuild.

## Before searching — check what's already there

1. Look at the active skill catalog in the current system prompt. Many needs (notes, reminders, weather, TTS, GitHub, Slack, …) may already be covered by a bundled or user-installed skill.
2. If anything plausibly matches, **invoke it instead** — don't reinstall.
3. Only proceed to external search when the gap is real.

## Search — pick the first available source

Registries are checked in order; use the first one whose CLI is installed. If none is installed, fall back to source 3.

### 1. ClawHub (`clawhub` CLI, registry: clawhub.com)

```bash
clawhub --version            # detect
clawhub search "<query>"     # keyword search
```

Install flow:

```bash
clawhub install <slug> --dir ~/.hope-agent/skills
```

### 2. Skillhub (`skillhub` CLI, Tencent-CDN registry — better for users in China)

```bash
skillhub --version           # detect
skillhub search "<query>"
skillhub install <slug>      # installs to current workspace ./skills by default
```

After `skillhub install`, if the slug landed under `./skills/<name>/`, move it to `~/.hope-agent/skills/<name>/` so Hope Agent's extra_skills_dir picks it up.

### 3. GitHub code search (fallback, no CLI required)

Use `gh` (already available on most machines) to search public repos for `SKILL.md` files:

```bash
gh api -X GET search/code \
  -f q='filename:SKILL.md "<keyword>"' \
  --jq '.items[] | {repo: .repository.full_name, path: .path, url: .html_url}' \
  | head -20
```

For promising hits, fetch the raw SKILL.md to read the frontmatter (`name`, `description`, `license`) before recommending.

## Quality gate — before recommending

Every candidate must pass:

- **License present and permissive** (MIT / Apache-2.0 / BSD / ISC / CC-BY). If the repo has no LICENSE file, flag it — user decides whether to accept the legal gray area.
- **SKILL.md frontmatter parses cleanly** (`name` + `description` minimum).
- **No obvious red flags** in bundled scripts: `curl … | sh`, network calls to unknown hosts, credential harvesting patterns. Skim scripts before install.
- **Install count / stars** if the registry exposes them (`clawhub search` prints `installs`): prefer >1k installs or >50 stars.

## Install — always confirm with the user first

This is a **HIGH-risk** operation (arbitrary third-party code joins the agent's toolchain). Workflow:

1. Present top 1–3 candidates to the user with: name, one-line description, source URL, license, install count / stars.
2. **Ask explicitly**: "Install `<slug>` from `<source>`? This adds it to `~/.hope-agent/skills/<name>/` and runs on future turns."
3. Only after explicit "yes" / "go ahead":
   - Run the registry CLI install, or `git clone` the repo into `~/.hope-agent/skills/<slug>/`.
   - If the skill's frontmatter declares `metadata.openclaw.install` or `metadata.hope-agent.install` with `bins:`, those dependencies need separate install (via brew/npm/go/uv) — Hope Agent's Skills panel has an "Install dependency" button, or use `ha-settings` to toggle `allow_remote_install` for HTTP-mode users.
4. Verify install: `ls ~/.hope-agent/skills/<slug>/SKILL.md` — confirm the file exists.

## Post-install

Tell the user:

> Installed `<slug>` into `~/.hope-agent/skills/<slug>/`. It will appear in the skill catalog on the next turn / conversation. Open **Settings → Skills** to review, or disable it if unwanted.

The catalog rebuilds automatically when the cache version bumps (add/remove/toggle triggers a bump). If the user doesn't see the new skill in the next turn, ask them to toggle it in Settings → Skills, which forces a refresh.

## What this skill will NOT do

- It won't install skill dependencies (brew / npm / go binaries) silently — those require explicit user consent via the Skills panel or `ha-settings`.
- It won't touch bundled skills (`skills/` in the app install) — those are vendored and versioned with the release.
- It won't delete installed skills. Use Settings → Skills or tell the user to `rm -rf ~/.hope-agent/skills/<slug>/`.

## Credits

Design inspired by [`vercel-labs/skills#find-skills`](https://github.com/vercel-labs/skills), which popularised the "agent finds its own skills" pattern. ClawHub (MIT, `openclaw/clawhub`) and Skillhub (Tencent-COS installer) are the two registries this workflow speaks to; neither source file has been vendored — the CLI usage documented above is factual interoperation notes.
