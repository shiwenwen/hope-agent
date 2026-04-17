#!/usr/bin/env python3
"""Skill scaffold for OpenComputer.

Creates a skill directory with a fully-commented SKILL.md template (every
frontmatter field the current runtime understands, with TODO placeholders)
plus any requested resource subdirectories. Default path follows the
OpenComputer convention — project cwd gets `.opencomputer/skills/<name>/`,
standalone invocation writes `~/.opencomputer/skills/<name>/`.

Usage:
    init_skill.py <skill-name> [--path <dir>] [--resources scripts,references,assets]
                  [--context fork|inline] [--user-invocable] [--install brew,node]
                  [--examples]

Run `--help` for the full option list.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

MAX_SKILL_NAME_LENGTH = 64
ALLOWED_RESOURCES = {"scripts", "references", "assets"}
ALLOWED_INSTALL_KINDS = {"brew", "node", "go", "uv", "download"}
ALLOWED_CONTEXTS = {"fork", "inline"}

SKILL_TEMPLATE = """---
name: {skill_name}
# Short "what it is" (what the skill does). Keep <= 80 chars if possible.
description: "TODO one-sentence summary of what this skill does"
# Optional: trigger hint rendered AFTER description in the catalog. Keep
# `description` terse and put "use when X / user mentions Y" here.
whenToUse: "TODO concrete trigger — e.g. user asks about PR review or CI status"

# Extra slash-command names. Canonical name comes from `name:` above;
# these add additional entry points. Remove this line for most skills.
# aliases: [alt-name-1, alt-name-2]

# Optional: prerequisite checks. The skill is hidden from the catalog
# until all required bins/env/config are present. Remove whole block if
# the skill has no external deps.
# requires:
#   bins: [git]           # all-of — every listed CLI must be on PATH
#   anyBins: [rg, grep]   # any-of — at least one must be on PATH
#   env: [MY_API_TOKEN]   # env vars that must be set and non-empty
#   os: [darwin, linux]   # restrict platforms
#   config: [webSearch.provider]  # AppConfig paths that must be truthy
# always: false           # true = skip prerequisite checks entirely
# primaryEnv: MY_API_TOKEN  # env var satisfied by the provider apiKey

{invocation_block}{execution_block}{install_block}
---

# {skill_title}

## Overview

[TODO: 1-2 sentences explaining what this skill enables and when it's the
right tool vs ad-hoc exec/read. Be concrete.]

## When to Use

[TODO: Plain-English trigger list — the model reads this when deciding
whether to activate. Mirror the frontmatter `whenToUse` and expand with
examples.]

## Workflow

[TODO: Pick ONE structure that fits:

  1. Workflow-based — sequential steps (best for builds, deployments,
     multi-stage processes). SKILL.md is Step 1 → Step 2 → Step 3.
  2. Task-based — capability menu (best for analysis tools offering
     several independent operations). Quick Start + Task Category 1/2/3.
  3. Reference-based — rules and specs (best for style guides, brand
     guidelines, coding standards). Overview + categorized references/.

Most skills mix patterns; start simple and refactor when the body gets
past ~500 lines.]

## Resources

[TODO: Only mention the resource dirs this skill actually needs.]

- `scripts/` — executable code the skill invokes via `exec`. Model does
  not need to read these into context to run them.
- `references/` — reference docs loaded on demand via `read`. Use for
  API docs, detailed workflows, or material the main SKILL.md is too
  short to include.
- `assets/` — files copied into output (templates, fonts, icons). Not
  loaded into context.

## What NOT to Include

- README / INSTALLATION_GUIDE / CHANGELOG — skills are instructions for
  an AI, not human documentation.
- Anything that duplicates what the model already knows (general coding
  style, well-known APIs).
"""



EXAMPLE_SCRIPT = '''#!/usr/bin/env python3
"""Example helper script for {skill_name}.

Scripts in skills/*/scripts/ are invoked by the model via the `exec` tool;
they do NOT need to be read into the model's context. Keep them small,
single-purpose, and deterministic. Replace this file or delete it when
the skill doesn't need a custom script.
"""


def main() -> int:
    print("Example script for {skill_name}. Replace with real logic.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
'''

EXAMPLE_REFERENCE = """# Reference: {skill_title}

Placeholder reference document. SKILL.md should tell the model when to
`read` this file — e.g. "When troubleshooting failed builds, read
`references/error-codes.md`."

Good reference-file candidates:
- API error tables
- Decision trees too long to inline in SKILL.md
- Multi-step playbooks invoked by specific triggers

Delete or rename when you add real content.
"""

EXAMPLE_ASSET = """# Example Asset

`assets/` holds files the skill emits as OUTPUT (templates, boilerplate,
fonts, icons). They are not loaded into the model's context.

Typical assets: .pptx templates, brand logos, starter project directories.
This text file is a placeholder — replace or delete.
"""


def normalize_skill_name(raw: str) -> str:
    """Lowercase + hyphenate + collapse repeats. Mirrors the Rust-side
    `normalize_skill_command_name` rules closely enough that the scaffolded
    skill's canonical slash command matches its directory name."""
    normalized = raw.strip().lower()
    normalized = re.sub(r"[^a-z0-9]+", "-", normalized)
    normalized = normalized.strip("-")
    normalized = re.sub(r"-{2,}", "-", normalized)
    return normalized


def title_case_skill_name(skill_name: str) -> str:
    return " ".join(word.capitalize() for word in skill_name.split("-"))


def parse_csv_option(raw: str, allowed: set[str], flag: str) -> list[str]:
    """Parse a comma-separated option, validating against `allowed`."""
    if not raw:
        return []
    items = [item.strip() for item in raw.split(",") if item.strip()]
    invalid = sorted({item for item in items if item not in allowed})
    if invalid:
        print(f"[ERROR] {flag}: unknown value(s): {', '.join(invalid)}")
        print(f"        Allowed: {', '.join(sorted(allowed))}")
        sys.exit(1)
    # De-dupe while preserving order.
    seen: set[str] = set()
    return [x for x in items if not (x in seen or seen.add(x))]


def default_skill_root() -> Path:
    """Pick a sensible output root when `--path` isn't given.

    Walk up from cwd looking for a `.git` / `Cargo.toml` / `package.json`
    marker — when found, use `<repo>/.opencomputer/skills/`. Otherwise fall
    back to `~/.opencomputer/skills/` so the skill lives in the user-level
    managed set instead of cluttering the current directory."""
    cwd = Path.cwd()
    for candidate in [cwd, *cwd.parents]:
        if (
            (candidate / ".git").exists()
            or (candidate / "Cargo.toml").exists()
            or (candidate / "package.json").exists()
        ):
            return candidate / ".opencomputer" / "skills"
    return Path.home() / ".opencomputer" / "skills"


def build_invocation_block(user_invocable: bool) -> str:
    if not user_invocable:
        return (
            "# Set `user-invocable: true` to register this skill as a slash\n"
            "# command (`/{skill-name}`). Only enable for skills the user is\n"
            "# expected to trigger directly.\n# user-invocable: false\n"
        )
    return (
        "# Slash-command registration. Remove `user-invocable` to hide from\n"
        "# the slash menu (model activation still works).\n"
        "user-invocable: true\n"
        "# Optional: hint shown next to the / command in the UI.\n"
        "# argumentHint: \"<query>\"\n"
    )


def build_execution_block(context: str | None) -> str:
    """Render the `context:` + `allowed-tools:` guidance block."""
    ctx_line = (
        f"context: {context}\n"
        if context
        else (
            "# Execution mode. `fork` runs the skill in a sub-agent (isolated\n"
            "# tool results, summary comes back to the main chat). `inline`\n"
            "# keeps everything in the main conversation. Prefer `fork` for\n"
            "# self-contained skills with lots of exec/read; `inline` when\n"
            "# the user needs to steer mid-flow.\n# context: inline\n"
        )
    )
    return (
        ctx_line
        + "# Restrict tools visible to this skill. Leave empty for full\n"
        + "# access. Typical minimum for a read-only skill: [read, grep, glob].\n"
        + "# allowed-tools: [read, grep, glob, exec]\n"
    )


def build_install_block(kinds: list[str]) -> str:
    if not kinds:
        return (
            "# Optional: auto-install deps before the skill runs. Stubs for\n"
            "# common package managers — fill in real formula/package names.\n"
            "# install:\n"
            "#   - kind: brew\n#     formula: gh\n#     bins: [gh]\n"
        )
    stubs = ["install:"]
    for kind in kinds:
        if kind == "brew":
            stubs.append("  - kind: brew\n    formula: TODO\n    bins: [TODO]")
        elif kind == "node":
            stubs.append("  - kind: node\n    package: TODO\n    bins: [TODO]")
        elif kind == "go":
            stubs.append("  - kind: go\n    module: github.com/TODO/tool@latest")
        elif kind == "uv":
            stubs.append("  - kind: uv\n    package: TODO")
        elif kind == "download":
            stubs.append("  - kind: download\n    # TODO: configure download spec")
    return "\n".join(stubs) + "\n"


def create_resource_dirs(
    skill_dir: Path,
    skill_name: str,
    skill_title: str,
    resources: list[str],
    include_examples: bool,
) -> None:
    for resource in resources:
        target = skill_dir / resource
        target.mkdir(exist_ok=True)
        if not include_examples:
            print(f"[OK] Created {resource}/")
            continue
        if resource == "scripts":
            path = target / "example.py"
            path.write_text(EXAMPLE_SCRIPT.format(skill_name=skill_name))
            path.chmod(0o755)
            print("[OK] Created scripts/example.py")
        elif resource == "references":
            path = target / "example.md"
            path.write_text(EXAMPLE_REFERENCE.format(skill_title=skill_title))
            print("[OK] Created references/example.md")
        elif resource == "assets":
            (target / "example.md").write_text(EXAMPLE_ASSET)
            print("[OK] Created assets/example.md")


def init_skill(
    skill_name: str,
    out_root: Path,
    resources: list[str],
    include_examples: bool,
    context: str | None,
    user_invocable: bool,
    install_kinds: list[str],
) -> Path | None:
    skill_dir = out_root.resolve() / skill_name
    if skill_dir.exists():
        print(f"[ERROR] Skill directory already exists: {skill_dir}")
        return None
    try:
        skill_dir.mkdir(parents=True, exist_ok=False)
    except OSError as exc:
        print(f"[ERROR] Could not create {skill_dir}: {exc}")
        return None
    print(f"[OK] Created skill directory: {skill_dir}")

    rendered = SKILL_TEMPLATE.format(
        skill_name=skill_name,
        skill_title=title_case_skill_name(skill_name),
        invocation_block=build_invocation_block(user_invocable),
        execution_block=build_execution_block(context),
        install_block=build_install_block(install_kinds),
    )
    (skill_dir / "SKILL.md").write_text(rendered)
    print("[OK] Created SKILL.md")

    if resources:
        create_resource_dirs(
            skill_dir,
            skill_name,
            title_case_skill_name(skill_name),
            resources,
            include_examples,
        )

    rel = os.path.relpath(skill_dir, Path.cwd())
    print(f"\n[OK] Skill '{skill_name}' scaffolded at {rel}")
    print("\nNext:")
    print("  1. Edit SKILL.md — fill TODOs, delete frontmatter lines you don't need.")
    print(f"  2. python scripts/quick_validate.py {rel}")
    print("  3. /skill-creator to iterate further (evals, description tuning).")
    return skill_dir


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Scaffold a new OpenComputer skill.",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument("skill_name", help="Skill name (normalized to hyphen-case)")
    parser.add_argument(
        "--path",
        default=None,
        help="Output root (parent of the new skill dir). "
        "Default: .opencomputer/skills in the nearest repo, else ~/.opencomputer/skills.",
    )
    parser.add_argument(
        "--resources",
        default="",
        help="Comma-separated subset of: scripts,references,assets",
    )
    parser.add_argument(
        "--context",
        choices=sorted(ALLOWED_CONTEXTS),
        default=None,
        help="Pre-fill `context:` frontmatter (else leave commented stub).",
    )
    parser.add_argument(
        "--user-invocable",
        action="store_true",
        help="Pre-fill `user-invocable: true` (register a /slash-command).",
    )
    parser.add_argument(
        "--install",
        default="",
        help="Comma-separated install kinds to stub: brew,node,go,uv,download",
    )
    parser.add_argument(
        "--examples",
        action="store_true",
        help="Create example.py / example.md files inside resource dirs.",
    )
    args = parser.parse_args()

    skill_name = normalize_skill_name(args.skill_name)
    if not skill_name:
        print("[ERROR] Skill name must include at least one letter or digit.")
        sys.exit(1)
    if len(skill_name) > MAX_SKILL_NAME_LENGTH:
        print(
            f"[ERROR] Skill name '{skill_name}' is {len(skill_name)} chars "
            f"(max {MAX_SKILL_NAME_LENGTH})."
        )
        sys.exit(1)
    if skill_name != args.skill_name.strip().lower():
        print(f"Note: normalized '{args.skill_name}' → '{skill_name}'.")

    resources = parse_csv_option(args.resources, ALLOWED_RESOURCES, "--resources")
    install_kinds = parse_csv_option(args.install, ALLOWED_INSTALL_KINDS, "--install")
    if args.examples and not resources:
        print("[ERROR] --examples requires --resources to be set.")
        sys.exit(1)

    out_root = Path(args.path) if args.path else default_skill_root()
    print(f"Initializing skill: {skill_name}")
    print(f"   Root: {out_root}")
    if resources:
        print(f"   Resources: {', '.join(resources)}")
    if args.context:
        print(f"   Context: {args.context}")
    if args.user_invocable:
        print("   User-invocable: true")
    if install_kinds:
        print(f"   Install stubs: {', '.join(install_kinds)}")
    print()

    result = init_skill(
        skill_name,
        out_root,
        resources,
        args.examples,
        args.context,
        args.user_invocable,
        install_kinds,
    )
    sys.exit(0 if result else 1)


if __name__ == "__main__":
    main()
