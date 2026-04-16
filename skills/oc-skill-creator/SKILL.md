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

1. **Metadata** (name + description) — always in context (~100 words)
2. **SKILL.md body** — loaded when the skill triggers (ideal <500 lines)
3. **Bundled resources** — loaded on demand (scripts can be executed directly, no need to read into context)

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
description: "Skill description..."      # Primary trigger mechanism — what it does + when to use

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
command-arg-placeholder: "<query>"      # UI placeholder hint
command-arg-options: [on, off]          # Fixed argument options
command-prompt-template: "..."          # Template with $ARGUMENTS expansion

# ── Optional: Execution Mode ──
context: inline                         # "fork" = sub-agent execution, "inline" = main conversation
allowed-tools: [exec, read, write]      # Tool whitelist during execution

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

### Body (Markdown)

Instructions the model reads after the skill triggers. Writing principles:

- **Use imperative mood**: tell the model directly what to do
- **Explain why, not pile on MUST**: the model is smart — understanding reasons works better than memorizing rules
- **Brevity first**: the context window is a shared resource, only write what the model doesn't already know
- **Examples beat lectures**: one good example is more effective than three paragraphs of explanation

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

- Ask about edge cases, input/output formats, success criteria
- Confirm prerequisites (which CLI tools, env vars are needed)
- Determine where to save the skill:
  - **Project-level** (`.opencomputer/skills/<name>/`) — workflows specific to this project
  - **User-level** (`~/.opencomputer/skills/<name>/`) — cross-project universal

### Step 3: Write SKILL.md

#### 3.1 Write frontmatter first

Determine name, description, and all needed fields. Description must be detailed enough to ensure correct triggering.

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

**Set appropriate degrees of freedom:**

- **High freedom** (text instructions): when multiple approaches work
- **Medium freedom** (pseudocode / parameterized scripts): preferred pattern but allow variation
- **Low freedom** (concrete scripts / steps): when operations are brittle, consistency is critical

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

For each test case, use `subagent` to launch two parallel runs:
1. **With skill**: read SKILL.md then execute the task
2. **Baseline**: execute without the skill

### Evaluate Results

1. **Grade**: use `agents/grader.md` instructions to evaluate each assertion
2. **Aggregate**: run `python scripts/aggregate_benchmark.py <workspace>/iteration-N --skill-name <name>`
3. **Analyze**: use `agents/analyzer.md` instructions to find patterns hidden in aggregate stats
4. **Visualize**: run `python eval-viewer/generate_review.py <workspace>/iteration-N --skill-name "my-skill"` to launch the browser viewer

### Iterative Improvement

When improving a skill based on user feedback:

1. **Generalize from feedback**: the skill will be used countless times — don't overfit to a few test cases
2. **Stay lean**: remove what doesn't work
3. **Explain why**: understand the reason behind user feedback, convey understanding rather than rigid rules
4. **Extract commonalities**: if multiple test cases write similar scripts, pre-package the script in `scripts/`

---

## Advanced: Blind A/B Testing

Use `agents/comparator.md` for blind A/B comparison: label two outputs as A and B without telling the judge which comes from which skill version. Then use `agents/analyzer.md` to analyze why the winner won.

This is optional — in most cases, a human review loop is sufficient.

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

- `agents/grader.md` — grading instructions for evaluating assertions
- `agents/comparator.md` — blind A/B comparison instructions
- `agents/analyzer.md` — analysis instructions for identifying winner reasons and improvement suggestions
- `references/schemas.md` — JSON schema definitions for evals.json, grading.json, benchmark.json, etc.

---

## What NOT to Include in Skills

Skills should only contain files the AI agent needs to complete its task. Do not create:
- README.md, INSTALLATION_GUIDE.md, CHANGELOG.md
- Documentation about the creation process
- User-facing installation guides
- Test procedure documentation

These only add clutter. Skills are for AI agents, not human-readable manuals.
