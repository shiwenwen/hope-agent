#!/usr/bin/env python3
"""Patch text in an existing PPTX while preserving its source package."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape


def replace_text_nodes(xml: str, replacements: list[dict]) -> tuple[str, int]:
    count = 0

    def replace_node(match: re.Match[str]) -> str:
        nonlocal count
        open_tag, text, close_tag = match.groups()
        updated = text
        for item in replacements:
            old = str(item.get("old", ""))
            new = str(item.get("new", ""))
            if old and old in updated:
                hits = updated.count(old)
                updated = updated.replace(old, new)
                count += hits
        return open_tag + escape(updated) + close_tag

    return re.sub(r"(<a:t>)(.*?)(</a:t>)", replace_node, xml), count


def patch_pptx(src: Path, patch: dict, out: Path) -> dict:
    replacements = patch.get("replace_text") or patch.get("replacements") or []
    if not replacements:
        raise SystemExit("Patch must include replace_text/replacements")
    total = 0
    touched = []
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if re.fullmatch(r"ppt/slides/slide\d+\.xml", item.filename):
                updated, count = replace_text_nodes(data.decode("utf-8"), replacements)
                if count:
                    total += count
                    touched.append(item.filename)
                    data = updated.encode("utf-8")
            zout.writestr(item, data)
    return {"input": str(src), "out": str(out), "replacements": total, "slides_touched": touched}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True)
    parser.add_argument("--patch", required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    patch = json.loads(Path(ns.patch).read_text(encoding="utf-8"))
    print(json.dumps(patch_pptx(Path(ns.input), patch, Path(ns.out)), indent=2))


if __name__ == "__main__":
    main()
