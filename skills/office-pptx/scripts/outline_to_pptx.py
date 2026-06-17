#!/usr/bin/env python3
"""Convert a compact Markdown outline into an editable PPTX deck."""

from __future__ import annotations

import argparse
from pathlib import Path

from build_pptx import write_pptx


def flush_slide(slides: list[dict], current: dict | None) -> None:
    if current and current.get("title"):
        if current.get("type") == "bullets" and not current.get("bullets") and current.get("body"):
            current["bullets"] = [current.pop("body")]
        slides.append(current)


def parse_outline(text: str) -> dict:
    deck_title = "Presentation"
    slides: list[dict] = []
    current: dict | None = None
    body_lines: list[str] = []

    def commit_body() -> None:
        nonlocal body_lines, current
        if current is not None and body_lines:
            current["body"] = " ".join(body_lines).strip()
        body_lines = []

    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        if line.startswith("# "):
            deck_title = line[2:].strip() or deck_title
            continue
        if line.startswith("## "):
            commit_body()
            flush_slide(slides, current)
            title = line[3:].strip()
            if title.lower().startswith("section:"):
                current = {"type": "section", "title": title.split(":", 1)[1].strip()}
            else:
                current = {"type": "bullets", "title": title, "bullets": []}
            continue
        if current is None:
            current = {"type": "bullets", "title": deck_title, "bullets": []}
        if line.startswith("- "):
            current.setdefault("bullets", []).append(line[2:].strip())
        elif line.startswith("* "):
            current.setdefault("bullets", []).append(line[2:].strip())
        else:
            body_lines.append(line)

    commit_body()
    flush_slide(slides, current)
    if not slides:
        slides = [{"type": "title", "title": deck_title}]
    else:
        slides.insert(0, {"type": "title", "title": deck_title})
    return {"title": deck_title, "slides": slides}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--outline", required=True, help="Markdown outline path")
    parser.add_argument("--out", required=True, help="Output .pptx path")
    ns = parser.parse_args()
    spec = parse_outline(Path(ns.outline).read_text(encoding="utf-8"))
    write_pptx(spec, Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
