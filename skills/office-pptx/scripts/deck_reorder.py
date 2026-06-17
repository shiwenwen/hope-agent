#!/usr/bin/env python3
"""Keep, drop, or reorder slides in a PPTX without rebuilding slide XML."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


SLIDE_ID_RE = re.compile(r'<p:sldId[^>]+r:id="(rId[0-9]+)"[^>]*/>')
REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"

ET.register_namespace("", REL_NS)


def parse_slide_rels(rels: str) -> dict[str, str]:
    root = ET.fromstring(rels)
    result = {}
    for rel in root:
        rel_type = rel.attrib.get("Type", "")
        target = rel.attrib.get("Target", "")
        match = re.fullmatch(r"slides/slide([0-9]+)\.xml", target)
        if rel_type.endswith("/slide") and match and rel.attrib.get("Id"):
            result[rel.attrib["Id"]] = match.group(1)
    return result


def filter_presentation_rels(rels: str, wanted_nums: set[str]) -> str:
    root = ET.fromstring(rels)
    for rel in list(root):
        rel_type = rel.attrib.get("Type", "")
        target = rel.attrib.get("Target", "")
        match = re.fullmatch(r"slides/slide([0-9]+)\.xml", target)
        if rel_type.endswith("/slide") and match and match.group(1) not in wanted_nums:
            root.remove(rel)
    return ET.tostring(root, encoding="utf-8", xml_declaration=True).decode("utf-8")


def reorder(src: Path, out: Path, order: list[int]) -> dict:
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin:
        presentation = zin.read("ppt/presentation.xml").decode("utf-8")
        rels = zin.read("ppt/_rels/presentation.xml.rels").decode("utf-8")
        rid_to_num = parse_slide_rels(rels)
        wanted_nums = {str(num) for num in order}
        old_ids = SLIDE_ID_RE.findall(presentation)
        rid_for_slide = {num: rid for rid, num in rid_to_num.items()}
        new_sld_ids = []
        next_id = 256
        for num in order:
            rid = rid_for_slide.get(str(num))
            if not rid:
                raise SystemExit(f"Slide not found: {num}")
            new_sld_ids.append(f'<p:sldId id="{next_id}" r:id="{rid}"/>')
            next_id += 1
        presentation = re.sub(r"<p:sldIdLst>.*?</p:sldIdLst>", "<p:sldIdLst>" + "".join(new_sld_ids) + "</p:sldIdLst>", presentation, flags=re.DOTALL)
        rels = filter_presentation_rels(rels, wanted_nums)
        with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
            for item in zin.infolist():
                slide_match = re.fullmatch(r"ppt/slides/slide([0-9]+)\.xml", item.filename)
                rel_match = re.fullmatch(r"ppt/slides/_rels/slide([0-9]+)\.xml\.rels", item.filename)
                if slide_match and slide_match.group(1) not in wanted_nums:
                    continue
                if rel_match and rel_match.group(1) not in wanted_nums:
                    continue
                if item.filename == "ppt/presentation.xml":
                    zout.writestr(item.filename, presentation)
                elif item.filename == "ppt/_rels/presentation.xml.rels":
                    zout.writestr(item.filename, rels)
                else:
                    zout.writestr(item, zin.read(item.filename))
    return {"input": str(src), "out": str(out), "order": order, "slide_count": len(order), "original_slide_ids": old_ids}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True)
    parser.add_argument("--order", required=True, help="JSON list such as [2,1,3]")
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    print(json.dumps(reorder(Path(ns.input), Path(ns.out), json.loads(ns.order)), indent=2))


if __name__ == "__main__":
    main()
