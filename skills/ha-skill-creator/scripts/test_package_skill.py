#!/usr/bin/env python3
"""Regression tests for package_skill.py.

Covers the happy path (zipping a valid skill), exclusion rules (build
artifacts / .DS_Store / root evals/), and the validation gate that must
block packaging when SKILL.md doesn't parse.
"""

from __future__ import annotations

import shutil
import sys
import tempfile
import zipfile
from pathlib import Path
from unittest import TestCase, main

HERE = Path(__file__).parent
# package_skill.py imports `from scripts.quick_validate import validate_skill`,
# which expects the skill-creator directory (parent of scripts/) on sys.path.
SKILL_ROOT = HERE.parent
for candidate in (str(SKILL_ROOT), str(HERE)):
    if candidate not in sys.path:
        sys.path.insert(0, candidate)

import package_skill as ps  # noqa: E402


MINIMAL_SKILL_MD = "---\nname: test-skill\ndescription: test\n---\n# Body\n"


def make_skill(root: Path, name: str = "test-skill") -> Path:
    skill = root / name
    skill.mkdir(parents=True, exist_ok=True)
    (skill / "SKILL.md").write_text(MINIMAL_SKILL_MD)
    (skill / "script.py").write_text("print('ok')\n")
    return skill


class TestPackageSkill(TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="test_pkg_"))

    def tearDown(self) -> None:
        if self.tmp.exists():
            shutil.rmtree(self.tmp)

    def test_packages_minimal_skill(self) -> None:
        skill = make_skill(self.tmp, "happy-skill")
        out = self.tmp / "out"
        out.mkdir()

        result = ps.package_skill(str(skill), str(out))

        self.assertIsNotNone(result)
        archive_path = out / "happy-skill.skill"
        self.assertTrue(archive_path.exists())
        with zipfile.ZipFile(archive_path) as z:
            names = set(z.namelist())
        self.assertIn("happy-skill/SKILL.md", names)
        self.assertIn("happy-skill/script.py", names)

    def test_excludes_build_artifacts(self) -> None:
        # __pycache__, *.pyc, and .DS_Store are never shipped.
        skill = make_skill(self.tmp, "with-garbage")
        (skill / "__pycache__").mkdir()
        (skill / "__pycache__" / "compiled.pyc").write_text("bytecode")
        (skill / "stale.pyc").write_text("bytecode")
        (skill / ".DS_Store").write_text("mac noise")

        out = self.tmp / "out"
        out.mkdir()
        result = ps.package_skill(str(skill), str(out))
        self.assertIsNotNone(result)

        with zipfile.ZipFile(out / "with-garbage.skill") as z:
            names = set(z.namelist())
        # Nothing from __pycache__, no loose .pyc, no .DS_Store.
        self.assertFalse(
            any("__pycache__" in n for n in names),
            f"__pycache__ leaked into archive: {names}",
        )
        self.assertFalse(any(n.endswith(".pyc") for n in names))
        self.assertFalse(any(n.endswith(".DS_Store") for n in names))

    def test_excludes_root_evals_dir(self) -> None:
        # evals/ at the skill root is dev-only scaffolding; nested evals/
        # directories should still ship (only ROOT level is excluded).
        skill = make_skill(self.tmp, "with-evals")
        (skill / "evals").mkdir()
        (skill / "evals" / "cases.json").write_text("[]")
        (skill / "references" / "nested" / "evals").mkdir(parents=True)
        (skill / "references" / "nested" / "evals" / "keeper.md").write_text("keep me")

        out = self.tmp / "out"
        out.mkdir()
        ps.package_skill(str(skill), str(out))
        with zipfile.ZipFile(out / "with-evals.skill") as z:
            names = set(z.namelist())
        self.assertFalse(any(n.startswith("with-evals/evals/") for n in names))
        self.assertIn(
            "with-evals/references/nested/evals/keeper.md",
            names,
            "Nested evals/ directories must still be packaged",
        )

    def test_validation_failure_blocks_packaging(self) -> None:
        skill = self.tmp / "broken"
        skill.mkdir()
        # Missing closing fence — quick_validate must reject.
        (skill / "SKILL.md").write_text(
            "---\nname: broken\ndescription: no closing fence\n"
        )
        out = self.tmp / "out"
        out.mkdir()

        result = ps.package_skill(str(skill), str(out))

        self.assertIsNone(result, "Invalid SKILL.md must block packaging")
        self.assertFalse(
            (out / "broken.skill").exists(),
            "No archive should be written when validation fails",
        )

    def test_missing_skill_md_fails_fast(self) -> None:
        skill = self.tmp / "no-skill-md"
        skill.mkdir()
        out = self.tmp / "out"
        out.mkdir()

        result = ps.package_skill(str(skill), str(out))
        self.assertIsNone(result)


if __name__ == "__main__":
    main()
