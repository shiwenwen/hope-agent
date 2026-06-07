#!/usr/bin/env python3
"""Sanitize DOCX title styling before Google Docs import."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path


TITLE_STYLE_RE = re.compile(r'<w:pStyle\s+w:val="Title"\s*/>')
PBDR_RE = re.compile(r"<w:pBdr>.*?</w:pBdr>", re.DOTALL)
UNDERLINE_RE = re.compile(r"<w:u\b[^>]*/>")
PARAGRAPH_RE = re.compile(r"<w:p\b.*?</w:p>", re.DOTALL)


def is_title_paragraph(paragraph: str) -> bool:
    return 'w:val="Title"' in paragraph or 'w:val="DocTitle"' in paragraph


def sanitize_document_xml(xml: str) -> tuple[str, dict]:
    stats = {"title_style_replacements": 0, "border_removals": 0, "underline_removals": 0}

    def replace_paragraph(match: re.Match[str]) -> str:
        paragraph = match.group(0)
        if not is_title_paragraph(paragraph):
            return paragraph
        stats["title_style_replacements"] += len(TITLE_STYLE_RE.findall(paragraph))
        stats["border_removals"] += len(PBDR_RE.findall(paragraph))
        stats["underline_removals"] += len(UNDERLINE_RE.findall(paragraph))
        paragraph = TITLE_STYLE_RE.sub('<w:pStyle w:val="DocTitle"/>', paragraph)
        paragraph = PBDR_RE.sub("", paragraph)
        paragraph = UNDERLINE_RE.sub("", paragraph)
        return paragraph

    xml = PARAGRAPH_RE.sub(replace_paragraph, xml)
    return xml, {
        "title_style_replacements": stats["title_style_replacements"],
        "border_removals": stats["border_removals"],
        "underline_removals": stats["underline_removals"],
    }


def sanitize_docx(src: Path, out: Path) -> dict:
    out.parent.mkdir(parents=True, exist_ok=True)
    stats = {"title_style_replacements": 0, "border_removals": 0, "underline_removals": 0}
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if item.filename == "word/document.xml":
                text, stats = sanitize_document_xml(data.decode("utf-8"))
                data = text.encode("utf-8")
            zout.writestr(item, data)
    return stats


def check_docx(path: Path) -> dict:
    with zipfile.ZipFile(path) as zf:
        xml = zf.read("word/document.xml").decode("utf-8")
    title_paragraphs = [match.group(0) for match in PARAGRAPH_RE.finditer(xml) if is_title_paragraph(match.group(0))]
    return {
        "has_builtin_title_style": any(TITLE_STYLE_RE.search(p) for p in title_paragraphs),
        "has_paragraph_borders": any(PBDR_RE.search(p) for p in title_paragraphs),
        "has_underline": any(UNDERLINE_RE.search(p) for p in title_paragraphs),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--out")
    parser.add_argument("--check", action="store_true")
    ns = parser.parse_args()
    src = Path(ns.input)
    if ns.check:
        result = check_docx(src)
        print(json.dumps(result, indent=2))
        if any(result.values()):
            raise SystemExit(1)
        return
    if not ns.out:
        raise SystemExit("--out is required unless --check is used")
    stats = sanitize_docx(src, Path(ns.out))
    print(json.dumps(stats, indent=2))


if __name__ == "__main__":
    main()
