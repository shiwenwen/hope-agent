---
name: oc-skill-creator
description: "Create, edit, improve, or audit OpenComputer skills. Use when the user wants to: (1) create a new skill from scratch, (2) edit or improve an existing skill, (3) review or clean up a SKILL.md file, (4) run evaluations to test skill effectiveness, (5) optimize skill descriptions for better trigger accuracy. Trigger phrases: 'create a skill', 'make a skill', 'improve this skill', 'review skill', 'audit skill'."
always: true
---

# Skill Creator

Tool for creating new skills and iteratively improving existing ones.

## Skill System Overview

OpenComputer skills are modular, self-contained packages that extend the AI assistant's capabilities with domain knowledge, workflows, and tools. Skills turn a general-purpose AI into a domain-specific expert.

### Skill Loading (Three-Tier Progressive Disclosure)

1. **Metadata** (name + description + whenToUse) — always in context (~100 words)
2. **SKILL.md body** — loaded when the skill triggers (ideal <500 lines)
3. **Bundled resources** — loaded on demand (scripts can be executed directly, no need to read into context)

### Body Organization — Three Common Patterns

Pick the pattern that matches the skill's shape. Most skills fit cleanly
into one; some mix patterns (e.g. start task-based, add a workflow for
the one complex operation). All three keep the body short by pushing
depth into `references/`.

**1. Workflow-based** — sequential process with ordered steps.
Best for builds, deployments, delivery pipelines, investigations.

```
SKILL.md
├── ## Overview
├── ## Step 1 — <setup>
├── ## Step 2 — <main action>
├── ## Step 3 — <verify / publish>
└── ## Troubleshooting   (refers to references/*.md per step)
```

**2. Task-based** — capability menu, operations are independent.
Best for analysis tools and skills offering several unrelated features.

```
SKILL.md
├── ## Overview
├── ## Quick Start
├── ## Task: <feature A>
├── ## Task: <feature B>
└── ## Task: <feature C>
```

**3. Reference-based** — specification / rules / standards.
Best for style guides, API schemas, brand rules.

```
SKILL.md
├── ## Overview
├── ## Core Rules
└── (detailed spec in references/<area>.md, loaded on demand)
```

### Skill Directory Structure

```
skill-name/
├── SKILL.md          (required: frontmatter + Markdown instructions)
├── scripts/          (optional: executable scripts, Python/Bash etc.)
├── references/       (optional: reference docs loaded on demand)
└── assets/           (optional: templates, icons, output materials)
```

### Skill Sources (lowest → highest precedence)

1. **Bundled** — shipped with OpenComputer, `skills/` directory
2. **Extra directories** — user-imported, `config.json` `extraSkillsDirs`
3. **Managed** — `~/.opencomputer/skills/`
4. **Project** — `.opencomputer/skills/` (relative to cwd, highest precedence)

---

## SKILL.md Format Specification

### Frontmatter (YAML)

```yaml
---
# ── Required ──
name: my-skill                          # Skill identifier (lowercase + hyphens)
description: "Short summary of what the skill does."
whenToUse: "Trigger hint — when the model should activate this skill"

# ── Optional: Identity ──
aliases: [alt-name-1, alt-name-2]       # Extra slash-command names for the same skill

# ── Optional: Prerequisites ──
requires:
  bins: [git, gh]                       # All must exist in PATH (AND)
  anyBins: [rg, grep]                   # At least one must exist (OR)
  env: [GITHUB_TOKEN]                   # Required environment variables
  os: [darwin, linux]                   # Supported operating systems
  config: [webSearch.provider]          # Config paths that must be truthy
always: false                           # true = skip all prerequisite checks
primaryEnv: MY_API_KEY                  # Primary env var (can be satisfied by apiKey)

# ── Optional: Invocation Control ──
user-invocable: true                    # Register as /command slash command
disable-model-invocation: false         # true = hide from model prompt directory
skillKey: custom-key                    # Custom config lookup key

# ── Optional: Command Dispatch ──
command-dispatch: tool                  # "tool" or "prompt"
command-tool: exec                      # Tool to bind when dispatch=tool
command-arg-mode: raw                   # Argument passing mode
argumentHint: "<query>"                 # UI placeholder hint (alias: command-arg-placeholder)
command-arg-options: [on, off]          # Fixed argument options
command-prompt-template: "..."          # Template with $ARGUMENTS expansion

# ── Optional: Execution Mode ──
context: inline                         # "fork" = sub-agent, "inline" = main conversation (see guidance below)
allowed-tools: [read, grep, glob]       # Tool whitelist during execution
agent: code-reviewer                    # Sub-agent type to use when context=fork (optional)
effort: medium                          # Reasoning effort for forked sub-agent (low|medium|high)

# ── Optional: Dependency Installation ──
install:
  - kind: brew
    formula: gh
    bins: [gh]
    label: "Install GitHub CLI (brew)"
    os: [darwin]
  - kind: node
    package: "@anthropic-ai/sdk"
    bins: [anthropic]
  - kind: go
    module: github.com/user/tool@latest
  - kind: uv
    package: my-python-tool
---
```

### Execution Mode — Fork vs Inline

`context:` decides where the skill runs. The choice matters: a wrong pick
either pollutes the main conversation with noisy tool output or hides
intermediate state the user needs to steer.

| Use `fork` (sub-agent) when | Use `inline` (main conversation) when |
|---|---|
| The skill runs many `exec` or `read` calls whose output is a one-time consumable | The user will react to intermediate output before the skill finishes |
| Work is self-contained — you can hand the caller a summary | You need `ask_user_question` inside the flow |
| Typical: builds, deployments, packaging, data pipelines | Typical: code review, interactive refactors, iterative writing |
| You explicitly want noise-isolated tool results | Tool calls are few and lightweight (1–3) |

Under the hood `fork` spawns a sub-agent with the skill's `allowed-tools`,
runs to completion, and injects **only the final summary** back into the
parent conversation. The parent's prompt cache stays clean; the sub-agent's
transcript is available on the session detail page but never re-enters
the main turn. `agent:` and `effort:` apply only when `context: fork`.

### Allowed Tools — How to Scope

Start with the smallest viable toolset. The default (empty = all tools)
is almost never right for a narrow skill — the wider the surface, the
more the sub-agent can drift.

| Skill archetype | Recommended `allowed-tools` |
|---|---|
| Read-only analysis (grep repo, summarize docs) | `[read, grep, glob]` |
| File-editing (apply fixes, refactor) | `[read, grep, glob, write, edit]` |
| Shell-heavy workflow (builds, deployments) | `[read, grep, glob, write, edit, exec]` + `context: fork` |
| Networked (web search / fetch) | add `[web_search, web_fetch]` on top of the archetype above |

Red lines:
- Do **not** include `subagent`, `team`, or `skill` — these are meta
  tools the skill itself shouldn't re-enter.
- Tool **pattern matching** (e.g. `exec(gh:*)`) is **not** supported yet.
  Whitelist is tool-name-only; finer-grained control requires skill-level
  wrapper scripts.

### Body (Markdown)

Instructions the model reads after the skill triggers. Writing principles:

- **Imperative mood**: tell the model directly what to do.
- **Explain why once**: one reason beats three MUSTs — the model is
  capable of generalizing from a principle, so don't repeat the same
  concept in every sub-section.
- **Conciseness**: the context window is a shared resource. A rule of
  thumb — if a paragraph is explaining something the model already
  knows (general coding style, widely-documented APIs), delete it.
- **Examples beat lectures**: one concrete example with realistic
  file paths and user requests outperforms three paragraphs of prose.

### Description Writing Guidelines

The description is the skill's **primary trigger mechanism** — the model decides whether to use the skill based on it.

- Clearly state what the skill **does** and **when to use it**
- All "when to use" info goes in the description, not the body (the body loads only after triggering)
- Be appropriately aggressive — avoid under-triggering. For example:

  Bad: `"GitHub operations tool"`
  Good: `"GitHub operations via gh CLI: issues, PRs, CI checks, code review. Use when the user mentions PR status, CI checks, creating issues, merge requests — even if they don't explicitly say 'GitHub'."`

---

## Skill Creation Flow

### Step 1: Understand Intent

Extract information from the current conversation, or ask to learn:

1. What should this skill enable the AI to do?
2. When should it trigger? (What will the user say?)
3. What's the expected output format?
4. Are test cases needed for validation?

If the conversation already contains a workflow (user says "turn this into a skill"), extract steps, tools used, user corrections, etc. from conversation history.

### Step 2: Interview & Research

- Ask about edge cases, input/output formats, success criteria.
- Confirm prerequisites (which CLI tools, env vars are needed).
- Decide between fork and inline execution — see [Execution Mode — Fork
  vs Inline](#execution-mode--fork-vs-inline) above. Default is inline
  unless the skill is shell-heavy or produces noisy intermediate output.
- Determine where to save the skill:
  - **Project-level** (`.opencomputer/skills/<name>/`) — workflows
    specific to this repo. Ship alongside the code that depends on them.
  - **User-level** (`~/.opencomputer/skills/<name>/`) — cross-project
    universal helpers (GitHub ops, favorite analysis workflows).

Scaffold the directory with the init helper (picks project vs user root
automatically based on whether you're inside a repo):

```bash
python skills/oc-skill-creator/scripts/init_skill.py my-skill \
  --resources scripts,references \
  --context fork \
  --examples
```

### Step 3: Write SKILL.md

#### 3.1 Write frontmatter first

Determine name, description, and the minimum set of extra fields.

**Naming conventions** (align with the skill-command normalizer):

- Lowercase ASCII letters, digits, and hyphens only. No underscores,
  no camelCase.
- Length ≤ 64 characters.
- Verb-led short phrase when possible (`review-pr`, not `pull-requests`).
- Namespace by the external tool or domain for related skills
  (`gh-*` for GitHub-specific, `ones-*` for ONES, `stlc-*` for client
  delivery). Makes the catalog scannable as it grows.
- The skill directory name must match `name:` exactly.

**Pick the minimum useful field set for your archetype** — there are
~20 frontmatter keys but most skills need 5–7:

| Archetype | Fields to fill |
|---|---|
| Minimal in-context skill | `name`, `description`, `whenToUse` |
| Slash command skill | + `user-invocable`, `argumentHint` |
| Depends on external CLI | + `requires.bins`, `install` (for auto-install) |
| Shell-heavy workflow | + `context: fork`, `allowed-tools` |
| Analysis-only skill | + `context: fork`, `allowed-tools: [read, grep, glob]` |

When in doubt, write less. Fields you don't set fall back to sensible
defaults; fields you do set must be kept accurate as the skill evolves.

Run `python scripts/init_skill.py <name>` to generate a skeleton with
every supported field present as a commented-out stub — delete the ones
you don't need rather than remembering which to add.

#### 3.2 Plan bundled resources

Analyze each use scenario:
- Code that would be written repeatedly? → put in `scripts/`
- Documentation the model needs to reference? → put in `references/`
- Templates needed in output? → put in `assets/`

#### 3.3 Write the body

Follow progressive disclosure:
- Keep SKILL.md under 500 lines
- Move large files to `references/` and specify in the body when to read them
- Keep reference files one level deep, referenced directly from SKILL.md

#### 3.4 Writing style

**Set appropriate degrees of freedom.** Think of each instruction like
a bridge: wide bridges let the model pick the best route; narrow bridges
with cliffs on either side (brittle shell commands, destructive data
migrations, APIs where order matters) must be rails, not suggestions.

- **High freedom** (plain-English instructions): multiple approaches
  work, the model can pick based on context.
- **Medium freedom** (pseudocode or parameterized scripts): preferred
  pattern, but the exact shape can vary with inputs.
- **Low freedom** (checked-in scripts, exact command strings): when
  one wrong step causes unrecoverable state. Ship the canonical command
  and tell the model to invoke it rather than re-compose the arguments.

When in doubt, widen. Over-specified skills age badly — every changed
flag, renamed tool, or updated API forces a skill update.

### Step 4: Confirm & Save

Before writing, show the complete SKILL.md content to the user as a yaml code block for review. After confirmation, write the file and tell the user:
- Where it was saved
- How to invoke it: `/<skill-name> [args]`
- They can edit SKILL.md directly to adjust

---

## Testing & Evaluation

### Write Test Cases

Create 2-3 realistic test prompts, save to `evals/evals.json`:

```json
{
  "skill_name": "my-skill",
  "evals": [
    {
      "id": 1,
      "prompt": "User's task description",
      "expected_output": "Description of expected result",
      "files": [],
      "expectations": [
        "Output contains X",
        "Used script Y"
      ]
    }
  ]
}
```

Full schema in `references/schemas.md`.

### Run Tests

Organize results in `<skill-name>-workspace/iteration-<N>/`.

For each test case, fan out **two parallel `subagent` runs** — both
share the same prompt so the comparison stays fair:

1. **With skill**: read `SKILL.md` first, then execute the task.
2. **Baseline**: execute the same prompt without loading the skill.

Running the two in parallel (not sequentially) matters: sequential runs
let later runs borrow context from earlier ones and mask regressions.

### Evaluate Results

1. **Grade**: use `agents/grader.md` instructions to evaluate each assertion
2. **Aggregate**: run `python scripts/aggregate_benchmark.py <workspace>/iteration-N --skill-name <name>`
3. **Analyze**: use `agents/analyzer.md` instructions to find patterns hidden in aggregate stats
4. **Visualize**: run `python eval-viewer/generate_review.py <workspace>/iteration-N --skill-name "my-skill"` to launch the browser viewer

### Iterative Improvement

When improving a skill based on user feedback:

1. **Generalize from feedback** — the skill will be used countless
   times, so avoid overfitting to the specific test case at hand.
2. **Stay lean** — remove what doesn't work before adding more rules.
3. **Extract commonalities** — if multiple test cases need similar
   helper code, pre-package it in `scripts/` rather than restating
   the snippet in every Step.

---

## Advanced: Blind A/B Testing

Use blind A/B when you can't trust yourself (or the user) to compare two
skill variants fairly. Typical triggers:

- Two variants produce superficially similar output — you need a neutral
  rubric to surface subtle differences.
- The author has sunk effort into variant B and would unconsciously bias
  an open comparison.
- You're optimizing against a subjective metric (tone, clarity,
  "feels done") where no assertion can fire.

Protocol:

1. Collect both runs' outputs into the same directory.
2. Relabel them as `A/` and `B/` before showing the comparator — the
   judge must not know which came from which variant.
3. Run [agents/comparator.md](agents/comparator.md). It scores each
   output against three dimensions (content quality, structure,
   completeness) and declares a winner with reasons.
4. Feed both the comparator verdict and the per-assertion grading into
   [agents/analyzer.md](agents/analyzer.md) to extract the **pattern**
   behind the win (e.g. "Winner always read `references/api.md` before
   calling `exec`"), which then becomes your next iteration target.

For non-subjective changes (a bug fix, a missing field) a single human
review loop is faster. Reserve blind A/B for "which phrasing works
better" style questions.

---

## Description Optimization

After completing a skill, optimize the description for better trigger accuracy:

1. **Generate trigger evaluation set**: create 20 queries (~10 should-trigger + ~10 should-not-trigger)
   - should-trigger: same intent in different phrasings, including cases that don't explicitly mention the skill name
   - should-not-trigger: similar but actually requiring different tools (harder = more valuable)
   - Queries should be specific and realistic, including file paths, personal context, etc.

2. **User review**: show the evaluation set for the user to confirm or modify

3. **Iterative optimization**: improve the description based on trigger test results until both should-trigger and should-not-trigger accuracy are satisfactory

---

## Reference Files

Agent prompts (loaded by the model on demand during evaluation):

- [`agents/grader.md`](agents/grader.md) — grading instructions for
  evaluating assertions with evidence extraction.
- [`agents/comparator.md`](agents/comparator.md) — blind A/B comparison
  protocol.
- [`agents/analyzer.md`](agents/analyzer.md) — post-hoc analysis for
  winner-pattern identification and improvement suggestions.

Schemas:

- [`references/schemas.md`](references/schemas.md) — JSON schemas for
  `evals.json`, `grading.json`, `benchmark.json`, and the comparison /
  analysis records.

Scripts (executable directly, no need to `read` into context):

- [`scripts/init_skill.py`](scripts/init_skill.py) — scaffold a new
  skill with full-frontmatter template + optional resource subdirs.
- [`scripts/quick_validate.py`](scripts/quick_validate.py) — frontmatter
  shape + kebab-case name + length checks.
- [`scripts/package_skill.py`](scripts/package_skill.py) — produce a
  `.skill` archive (zip) for distribution, runs validation first.
- [`scripts/run_eval.py`](scripts/run_eval.py) +
  [`run_loop.py`](scripts/run_loop.py) — dispatch eval runs via
  sub-agent.
- [`scripts/aggregate_benchmark.py`](scripts/aggregate_benchmark.py) —
  combine per-case grading into benchmark stats.
- [`scripts/improve_description.py`](scripts/improve_description.py) —
  optimize description for trigger accuracy.
- [`scripts/generate_report.py`](scripts/generate_report.py) +
  [`eval-viewer/generate_review.py`](eval-viewer/generate_review.py) —
  render HTML review of an iteration.
- [`scripts/test_quick_validate.py`](scripts/test_quick_validate.py) +
  [`scripts/test_package_skill.py`](scripts/test_package_skill.py) —
  self-tests; run via `python -m unittest` from inside `scripts/`.

---

## What NOT to Include in Skills

Skills should only contain files the AI agent needs to complete its task. Do not create:
- README.md, INSTALLATION_GUIDE.md, CHANGELOG.md
- Documentation about the creation process
- User-facing installation guides
- Test procedure documentation

These only add clutter. Skills are for AI agents, not human-readable manuals.
