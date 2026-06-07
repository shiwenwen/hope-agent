#!/usr/bin/env python3
"""Audit bundled Office skills against the primary-runtime Office skill surfaces."""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


REQUIREMENTS = {
    "office-docx": {
        "skill": "skills/office-docx/SKILL.md",
        "files": [
            "scripts/build_docx.py",
            "scripts/append_docx.py",
            "scripts/markdown_to_docx.py",
            "scripts/inspect_docx.py",
            "scripts/render_preview.py",
            "scripts/a11y_audit.py",
            "scripts/google_docs_title_sanitize.py",
            "scripts/comments_extract.py",
            "scripts/comments_strip.py",
            "scripts/add_tracked_replacements.py",
            "scripts/accept_tracked_changes.py",
            "scripts/redact_docx.py",
            "scripts/privacy_scrub_metadata.py",
            "scripts/compare_docx.py",
            "scripts/insert_toc.py",
            "scripts/fields_report.py",
            "scripts/insert_note.py",
            "scripts/watermark_add.py",
            "scripts/watermark_audit_remove.py",
            "scripts/set_protection.py",
            "scripts/content_controls.py",
            "scripts/internal_nav.py",
            "scripts/merge_docx_append.py",
            "scripts/docx_table_to_csv.py",
        ],
        "phrases": [
            "real Word numbering",
            "images with alt text",
            "Google Docs-targeted",
            "tracked changes",
            "render previews",
        ],
    },
    "office-xlsx": {
        "skill": "skills/office-xlsx/SKILL.md",
        "files": [
            "scripts/build_xlsx.py",
            "scripts/csv_to_xlsx.py",
            "scripts/patch_xlsx.py",
            "scripts/inspect_xlsx.py",
            "scripts/formula_audit.py",
            "scripts/recalculate_xlsx.py",
            "scripts/render_preview.py",
        ],
        "phrases": [
            "bar/column/line/pie charts",
            "real Excel tables",
            "data validation",
            "conditional formatting",
            "LibreOffice recalc",
        ],
    },
    "office-pptx": {
        "skill": "skills/office-pptx/SKILL.md",
        "files": [
            "scripts/build_pptx.py",
            "scripts/outline_to_pptx.py",
            "scripts/append_pptx.py",
            "scripts/patch_pptx.py",
            "scripts/duplicate_slide.py",
            "scripts/deck_reorder.py",
            "scripts/inspect_pptx.py",
            "scripts/layout_audit.py",
            "scripts/make_contact_sheet.py",
            "scripts/render_preview.py",
        ],
        "phrases": [
            "native PowerPoint chart objects",
            "template-following",
            "contact sheet",
            "layout audit",
            "preserve source-deck style",
        ],
    },
}


def audit_skill(name: str, spec: dict) -> dict:
    skill_dir = ROOT / "skills" / name
    skill_path = ROOT / spec["skill"]
    missing_files = [path for path in spec["files"] if not (skill_dir / path).exists()]
    body = skill_path.read_text(encoding="utf-8") if skill_path.exists() else ""
    lower_body = body.lower()
    missing_phrases = [phrase for phrase in spec["phrases"] if phrase.lower() not in lower_body]
    return {
        "skill": name,
        "passed": not missing_files and not missing_phrases,
        "missing_files": missing_files,
        "missing_phrases": missing_phrases,
        "file_count": len(spec["files"]),
    }


def main() -> None:
    results = [audit_skill(name, spec) for name, spec in REQUIREMENTS.items()]
    report = {
        "passed": all(item["passed"] for item in results),
        "skills": results,
    }
    print(json.dumps(report, indent=2))
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
