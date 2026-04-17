#!/usr/bin/env python3
"""Regression tests for quick_validate.py.

Verifies the validator accepts every frontmatter field the runtime knows
about (the allowlist in quick_validate.py must stay in sync with the Rust
parser in crates/oc-core/src/skills/frontmatter.rs) and rejects shapes
that would fail at load time.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest import TestCase, main

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))
import quick_validate  # noqa: E402  — must follow sys.path mutation


def write_skill(skill_dir: Path, body: str) -> None:
    skill_dir.mkdir(parents=True, exist_ok=True)
    (skill_dir / "SKILL.md").write_text(body, encoding="utf-8")


class TestQuickValidateShape(TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="test_qv_"))

    def tearDown(self) -> None:
        if self.tmp.exists():
            shutil.rmtree(self.tmp)

    def test_accepts_minimal_skill(self) -> None:
        skill = self.tmp / "minimal"
        write_skill(skill, "---\nname: minimal\ndescription: ok\n---\n# Body\n")
        ok, msg = quick_validate.validate_skill(skill)
        self.assertTrue(ok, msg)

    def test_accepts_crlf_frontmatter(self) -> None:
        skill = self.tmp / "crlf"
        write_skill(
            skill,
            "---\r\nname: crlf\r\ndescription: ok\r\n---\r\n# Body\r\n",
        )
        ok, msg = quick_validate.validate_skill(skill)
        self.assertTrue(ok, msg)

    def test_rejects_missing_closing_fence(self) -> None:
        skill = self.tmp / "no-fence"
        write_skill(
            skill,
            "---\nname: no-fence\ndescription: missing end\n# no closing fence\n",
        )
        ok, msg = quick_validate.validate_skill(skill)
        self.assertFalse(ok)
        self.assertIn("frontmatter", msg.lower())

    def test_rejects_missing_name(self) -> None:
        skill = self.tmp / "no-name"
        write_skill(skill, "---\ndescription: anonymous\n---\nBody\n")
        ok, msg = quick_validate.validate_skill(skill)
        self.assertFalse(ok)
        self.assertIn("name", msg.lower())

    def test_rejects_description_as_list(self) -> None:
        # Regression: scaffold templates must quote placeholders; unquoted
        # `[TODO: ...]` is parsed as a list and must fail fast with a clear
        # type error rather than getting past validation.
        skill = self.tmp / "list-desc"
        write_skill(
            skill,
            "---\nname: list-desc\ndescription: [TODO: fill me]\n---\nBody\n",
        )
        ok, msg = quick_validate.validate_skill(skill)
        self.assertFalse(ok)
        self.assertIn("string", msg.lower())

    def test_rejects_bad_name_convention(self) -> None:
        skill = self.tmp / "bad-name"
        write_skill(skill, "---\nname: Bad_Name\ndescription: x\n---\nBody\n")
        ok, msg = quick_validate.validate_skill(skill)
        self.assertFalse(ok)
        self.assertIn("kebab-case", msg.lower())


class TestQuickValidateExtendedSchema(TestCase):
    """Every field the Rust parser understands must pass validation."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="test_qv_ext_"))

    def tearDown(self) -> None:
        if self.tmp.exists():
            shutil.rmtree(self.tmp)

    def test_accepts_full_frontmatter(self) -> None:
        skill = self.tmp / "full"
        write_skill(
            skill,
            """---
name: full
description: "exercises every supported key"
whenToUse: "when the user asks about the full schema"
aliases: [full-alt, full2]
user-invocable: true
disable-model-invocation: false
skillKey: full-key
command-dispatch: tool
command-tool: exec
command-arg-mode: raw
command-arg-placeholder: "<query>"
argumentHint: "<query>"
command-arg-options: [on, off]
command-prompt-template: "Run $ARGUMENTS"
context: fork
agent: default
effort: medium
paths: ["src/**/*.rs"]
allowed-tools: [read, grep]
requires:
  bins: [git]
  anyBins: [rg, grep]
  env: [SOME_TOKEN]
  os: [darwin, linux]
  config: [webSearch.provider]
always: false
primaryEnv: SOME_TOKEN
install:
  - kind: brew
    formula: gh
    bins: [gh]
status: active
authored-by: test
rationale: "covers all fields"
---

Body
""",
        )
        ok, msg = quick_validate.validate_skill(skill)
        self.assertTrue(ok, f"Full schema must validate; got: {msg}")

    def test_init_skill_output_passes_validation(self) -> None:
        # Full end-to-end: scaffold with init_skill.py, validate result.
        target_root = self.tmp / "scaffold-root"
        result = subprocess.run(
            [
                sys.executable,
                str(HERE / "init_skill.py"),
                "integration-skill",
                "--path",
                str(target_root),
                "--resources",
                "scripts,references",
                "--context",
                "fork",
                "--user-invocable",
                "--install",
                "brew",
                "--examples",
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("Created SKILL.md", result.stdout)
        ok, msg = quick_validate.validate_skill(target_root / "integration-skill")
        self.assertTrue(ok, f"Scaffolded skill must validate; got: {msg}")


if __name__ == "__main__":
    main()
