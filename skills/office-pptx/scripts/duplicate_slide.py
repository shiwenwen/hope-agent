#!/usr/bin/env python3
"""Duplicate one slide in a PPTX package, preserving its rels and assets."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path


def next_num(names: set[str], pattern: str) -> int:
    nums = []
    for name in names:
        match = re.fullmatch(pattern, name)
        if match:
            nums.append(int(match.group(1)))
    return max(nums, default=0) + 1


def duplicate(src: Path, out: Path, slide: int) -> dict:
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin:
        names = set(zin.namelist())
        source = f"ppt/slides/slide{slide}.xml"
        if source not in names:
            raise SystemExit(f"Slide not found: {slide}")
        source_bytes = zin.read(source)
        rel_source = f"ppt/slides/_rels/slide{slide}.xml.rels"
        rel_source_bytes = zin.read(rel_source) if rel_source in names else None
        new_slide = next_num(names, r"ppt/slides/slide([0-9]+)\.xml")
        presentation = zin.read("ppt/presentation.xml").decode("utf-8")
        rels = zin.read("ppt/_rels/presentation.xml.rels").decode("utf-8")
        rel_ids = [int(v) for v in re.findall(r'Id="rId([0-9]+)"', rels)]
        new_rid = f"rId{max(rel_ids, default=0) + 1}"
        slide_ids = [int(v) for v in re.findall(r"<p:sldId[^>]+id=\"([0-9]+)\"", presentation)]
        new_sid = max(slide_ids, default=255) + 1
        presentation = presentation.replace("</p:sldIdLst>", f'<p:sldId id="{new_sid}" r:id="{new_rid}"/></p:sldIdLst>')
        rels = rels.replace("</Relationships>", f'<Relationship Id="{new_rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{new_slide}.xml"/></Relationships>')
        content_types = zin.read("[Content_Types].xml").decode("utf-8")
        content_types = content_types.replace("</Types>", f'<Override PartName="/ppt/slides/slide{new_slide}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>')
        with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
            for item in zin.infolist():
                if item.filename == "ppt/presentation.xml":
                    zout.writestr(item.filename, presentation)
                elif item.filename == "ppt/_rels/presentation.xml.rels":
                    zout.writestr(item.filename, rels)
                elif item.filename == "[Content_Types].xml":
                    zout.writestr(item.filename, content_types)
                else:
                    zout.writestr(item.filename, zin.read(item.filename))
            zout.writestr(f"ppt/slides/slide{new_slide}.xml", source_bytes)
            if rel_source_bytes is not None:
                zout.writestr(f"ppt/slides/_rels/slide{new_slide}.xml.rels", rel_source_bytes)
    return {"input": str(src), "out": str(out), "source_slide": slide, "new_slide": new_slide}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True)
    parser.add_argument("--slide", type=int, required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    print(json.dumps(duplicate(Path(ns.input), Path(ns.out), ns.slide), indent=2))


if __name__ == "__main__":
    main()
