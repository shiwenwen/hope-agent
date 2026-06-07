#!/usr/bin/env python3
"""Convert a practical Markdown subset into an editable DOCX."""

from __future__ import annotations

import argparse
from pathlib import Path

from build_docx import write_docx


def flush_paragraph(blocks: list[dict], lines: list[str]) -> None:
    if lines:
        blocks.append({"type": "paragraph", "text": " ".join(lines).strip()})
        lines.clear()


def parse_markdown(text: str) -> dict:
    title = "Document"
    blocks: list[dict] = []
    paragraph_lines: list[str] = []
    list_items: list[str] = []
    ordered_items: list[str] = []

    def flush_lists() -> None:
        if list_items:
            blocks.append({"type": "bullet_list", "items": list_items.copy()})
            list_items.clear()
        if ordered_items:
            blocks.append({"type": "numbered_list", "items": ordered_items.copy()})
            ordered_items.clear()

    lines = text.splitlines()
    for raw in lines:
        line = raw.strip()
        if not line:
            flush_paragraph(blocks, paragraph_lines)
            flush_lists()
            continue
        if line.startswith("# "):
            flush_paragraph(blocks, paragraph_lines)
            flush_lists()
            title = line[2:].strip() or title
            continue
        if line.startswith("## "):
            flush_paragraph(blocks, paragraph_lines)
            flush_lists()
            blocks.append({"type": "heading", "level": 1, "text": line[3:].strip()})
            continue
        if line.startswith("### "):
            flush_paragraph(blocks, paragraph_lines)
            flush_lists()
            blocks.append({"type": "heading", "level": 2, "text": line[4:].strip()})
            continue
        if line.startswith("> "):
            flush_paragraph(blocks, paragraph_lines)
            flush_lists()
            blocks.append({"type": "callout", "label": "Note", "text": line[2:].strip()})
            continue
        if line.startswith("- ") or line.startswith("* "):
            flush_paragraph(blocks, paragraph_lines)
            ordered_items.clear()
            list_items.append(line[2:].strip())
            continue
        if len(line) > 3 and line[0].isdigit() and ". " in line[:5]:
            flush_paragraph(blocks, paragraph_lines)
            list_items.clear()
            ordered_items.append(line.split(". ", 1)[1].strip())
            continue
        if "|" in line and line.startswith("|") and line.endswith("|"):
            flush_paragraph(blocks, paragraph_lines)
            flush_lists()
            cells = [cell.strip() for cell in line.strip("|").split("|")]
            if not cells or all(set(cell) <= {"-", ":"} for cell in cells):
                continue
            if blocks and blocks[-1].get("type") == "table":
                blocks[-1].setdefault("rows", []).append(cells)
            else:
                blocks.append({"type": "table", "headers": cells, "rows": []})
            continue
        paragraph_lines.append(line)

    flush_paragraph(blocks, paragraph_lines)
    flush_lists()
    return {"title": title, "blocks": blocks}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--markdown", required=True, help="Markdown input path")
    parser.add_argument("--out", required=True, help="Output .docx path")
    ns = parser.parse_args()
    spec = parse_markdown(Path(ns.markdown).read_text(encoding="utf-8"))
    write_docx(spec, Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
