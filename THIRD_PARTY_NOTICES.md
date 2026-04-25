# Third-Party Notices

Hope Agent ships with skill content adapted from third-party open-source projects. This file documents those vendored components and their original licenses. Per-skill `ATTRIBUTION.md` files reference back to this document.

---

## Vendored Skills (`skills/`)

The following skills under [`skills/`](./skills/) are adapted from upstream MIT-licensed projects. Adaptations include tool-name remapping (e.g. `read_file` → `read`, `delegate_task` → `subagent`), addition of `paths:` filter for context-aware activation, and minor stylistic edits.

| Skill | Source |
|---|---|
| [`skills/systematic-debugging/`](./skills/systematic-debugging/) | hermes-agent · skills/software-development/systematic-debugging/ |
| [`skills/test-driven-development/`](./skills/test-driven-development/) | hermes-agent · skills/software-development/test-driven-development/ |
| [`skills/writing-plans/`](./skills/writing-plans/) | hermes-agent · skills/software-development/writing-plans/ |
| [`skills/code-review/`](./skills/code-review/) | hermes-agent · skills/software-development/requesting-code-review/ |
| [`skills/subagent-driven-development/`](./skills/subagent-driven-development/) | hermes-agent · skills/software-development/subagent-driven-development/ |

### Hermes Agent

Source: <https://github.com/NousResearch/hermes>

Hermes Agent itself credits [obra/superpowers](https://github.com/obra/superpowers) as the original source for several of these skills.

```
MIT License

Copyright (c) 2025 Nous Research

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

---

## Quick Import Sources

Hope Agent's Skills setting (and the equivalent step in Onboarding) offers a "Quick Import" button that scans common locations for third-party skill catalogs and adds them to `extra_skills_dirs`. Hope Agent **does not vendor** these projects — content is loaded at runtime from the user's own machine if present.

| Source | Origin |
|---|---|
| Claude Code user-level skills (`~/.claude/skills/`) | Anthropic Claude Code |
| Claude Code plugin skills (`~/.claude/plugins/*/skills/`) | Various Claude Code plugins |
| Anthropic Agent Skills marketplace (`~/.claude/plugins/marketplaces/anthropic-agent-skills/`) | Anthropic, PBC — Proprietary; governed by Anthropic Consumer/Commercial Terms |
| OpenClaw skills (`~/Codes/openclaw/skills/`, etc.) | <https://github.com/> · MIT |
| Hermes Agent skills (`~/Codes/hermes-agent/skills/`, etc.) | <https://github.com/NousResearch/hermes> · MIT |

When Quick Import surfaces the Anthropic marketplace path, the UI shows a license-warning indicator. Use is governed by your agreement with Anthropic.
