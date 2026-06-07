#!/usr/bin/env python3
"""Run structural accessibility and delivery checks for DOCX files."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


def tag_ends(node: ET.Element, suffix: str) -> bool:
    return node.tag.endswith(suffix)


def paragraph_text(p: ET.Element) -> str:
    parts = []
    for node in p.iter():
        if tag_ends(node, "}t") or tag_ends(node, "}delText"):
            if node.text:
                parts.append(node.text)
    return "".join(parts)


def paragraph_style(p: ET.Element) -> str | None:
    for node in p.iter():
        if tag_ends(node, "}pStyle"):
            return node.attrib.get("{http://schemas.openxmlformats.org/wordprocessingml/2006/main}val")
    return None


def has_num_pr(p: ET.Element) -> bool:
    return any(tag_ends(node, "}numPr") for node in p.iter())


def audit(path: Path) -> dict:
    issues: list[dict] = []
    warnings: list[dict] = []
    with zipfile.ZipFile(path) as zf:
        names = set(zf.namelist())
        if "word/document.xml" not in names:
            return {"passed": False, "issues": [{"code": "missing_document_xml"}], "warnings": []}
        document = ET.fromstring(zf.read("word/document.xml"))
        paragraphs = [node for node in document.iter() if tag_ends(node, "}p")]
        tables = [node for node in document.iter() if tag_ends(node, "}tbl")]
        text = "\n".join(paragraph_text(p) for p in paragraphs).strip()
        heading_levels = []
        fake_bullets = []
        title_count = 0
        for idx, p in enumerate(paragraphs, 1):
            style = paragraph_style(p) or ""
            content = paragraph_text(p).strip()
            if style in {"Title", "DocTitle"}:
                title_count += 1
            match = re.fullmatch(r"Heading([1-6])", style)
            if match:
                heading_levels.append((idx, int(match.group(1))))
            if content.startswith(("• ", "- ", "* ")) and not has_num_pr(p):
                fake_bullets.append(idx)
        if not text:
            issues.append({"code": "empty_document", "message": "Document has no readable text."})
        if title_count == 0:
            warnings.append({"code": "missing_title_style", "message": "No title-style paragraph found."})
        if fake_bullets:
            issues.append({"code": "fake_bullets", "paragraphs": fake_bullets[:20]})
        previous = 0
        skipped = []
        for paragraph_idx, level in heading_levels:
            if previous and level > previous + 1:
                skipped.append({"paragraph": paragraph_idx, "from": previous, "to": level})
            previous = level
        if skipped:
            warnings.append({"code": "heading_level_skip", "items": skipped})
        for table_idx, table in enumerate(tables, 1):
            has_grid = any(tag_ends(node, "}tblGrid") for node in table.iter())
            if not has_grid:
                warnings.append({"code": "table_missing_grid", "table": table_idx})
        media = [name for name in names if name.startswith("word/media/")]
        drawing_count = sum(1 for node in document.iter() if tag_ends(node, "}drawing"))
        if media and drawing_count == 0:
            warnings.append({"code": "media_without_drawings", "media_count": len(media)})
        missing_alt = []
        for idx, node in enumerate(document.iter(), 1):
            if tag_ends(node, "}docPr") and not (node.attrib.get("descr") or "").strip():
                missing_alt.append(idx)
        if missing_alt:
            issues.append({"code": "image_missing_alt_text", "items": missing_alt[:20]})
        google_docs = {
            "has_builtin_title_style": 'w:val="Title"' in zf.read("word/document.xml").decode("utf-8"),
            "has_paragraph_borders": "<w:pBdr>" in zf.read("word/document.xml").decode("utf-8"),
        }
        if google_docs["has_builtin_title_style"] or google_docs["has_paragraph_borders"]:
            warnings.append({"code": "google_docs_title_sanitize_recommended", **google_docs})
    return {
        "path": str(path),
        "passed": not issues,
        "issues": issues,
        "warnings": warnings,
        "stats": {
            "paragraphs": len(paragraphs),
            "headings": len(heading_levels),
            "tables": len(tables),
            "title_paragraphs": title_count,
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("path")
    parser.add_argument("--fail-on-warning", action="store_true")
    ns = parser.parse_args()
    result = audit(Path(ns.path))
    print(json.dumps(result, ensure_ascii=False, indent=2))
    if not result["passed"] or (ns.fail_on_warning and result["warnings"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
