#!/usr/bin/env python3
"""Compare DOCX text content and emit a compact unified diff."""

from __future__ import annotations

import argparse
import difflib
import json
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


def extract_lines(path: Path) -> list[str]:
    with zipfile.ZipFile(path) as zf:
        root = ET.fromstring(zf.read("word/document.xml"))
    lines = []
    current = []
    for node in root.iter():
        if node.tag.endswith("}t") and node.text:
            current.append(node.text)
        elif node.tag.endswith("}p"):
            if current:
                lines.append("".join(current))
                current = []
    if current:
        lines.append("".join(current))
    return lines


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("before")
    parser.add_argument("after")
    parser.add_argument("--max-lines", type=int, default=200)
    ns = parser.parse_args()
    before = extract_lines(Path(ns.before))
    after = extract_lines(Path(ns.after))
    diff = list(
        difflib.unified_diff(
            before,
            after,
            fromfile=str(ns.before),
            tofile=str(ns.after),
            lineterm="",
        )
    )
    print(
        json.dumps(
            {
                "before_lines": len(before),
                "after_lines": len(after),
                "changed": before != after,
                "diff": diff[: ns.max_lines],
                "truncated": len(diff) > ns.max_lines,
            },
            ensure_ascii=False,
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
