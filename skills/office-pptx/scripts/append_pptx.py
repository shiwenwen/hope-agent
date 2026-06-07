#!/usr/bin/env python3
"""Append generated slides to an existing PPTX deck."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path

from build_pptx import native_chart_xml, slide_rels, slide_xml


def next_slide_number(names: set[str]) -> int:
    nums = []
    for name in names:
        match = re.fullmatch(r"ppt/slides/slide([0-9]+)\.xml", name)
        if match:
            nums.append(int(match.group(1)))
    return max(nums, default=0) + 1


def next_chart_number(names: set[str]) -> int:
    nums = []
    for name in names:
        match = re.fullmatch(r"ppt/charts/chart([0-9]+)\.xml", name)
        if match:
            nums.append(int(match.group(1)))
    return max(nums, default=0) + 1


def next_rel_id(rels_xml: str) -> int:
    ids = [int(value) for value in re.findall(r'Id="rId([0-9]+)"', rels_xml)]
    return max(ids, default=0) + 1


def next_slide_id(presentation_xml: str) -> int:
    ids = [int(value) for value in re.findall(r"<p:sldId[^>]+id=\"([0-9]+)\"", presentation_xml)]
    return max(ids, default=255) + 1


def add_content_type(content_types: str, slide_no: int) -> str:
    part = f"/ppt/slides/slide{slide_no}.xml"
    if part in content_types:
        return content_types
    override = (
        f'<Override PartName="{part}" '
        'ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>'
    )
    return content_types.replace("</Types>", override + "</Types>")


def add_chart_content_type(content_types: str, chart_no: int) -> str:
    part = f"/ppt/charts/chart{chart_no}.xml"
    if part in content_types:
        return content_types
    override = (
        f'<Override PartName="{part}" '
        'ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>'
    )
    return content_types.replace("</Types>", override + "</Types>")


def append_pptx(src: Path, spec: dict, out: Path) -> None:
    slides = spec.get("slides") or []
    if not slides:
        raise SystemExit("No slides in spec")
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin:
        names = set(zin.namelist())
        presentation = zin.read("ppt/presentation.xml").decode("utf-8")
        rels = zin.read("ppt/_rels/presentation.xml.rels").decode("utf-8")
        content_types = zin.read("[Content_Types].xml").decode("utf-8")
        slide_no = next_slide_number(names)
        chart_no = next_chart_number(names)
        slide_id = next_slide_id(presentation)
        rel_id = next_rel_id(rels)
        new_entries: dict[str, str] = {}
        next_chart_no = chart_no

        for idx, slide in enumerate(slides):
            current_slide_no = slide_no + idx
            current_rel_id = rel_id + idx
            current_slide_id = slide_id + idx
            chart_target = None
            chart_rel_id = None
            if slide.get("type") == "native_chart":
                chart_name = f"chart{next_chart_no}.xml"
                new_entries[f"ppt/charts/{chart_name}"] = native_chart_xml(slide)
                chart_target = f"../charts/{chart_name}"
                chart_rel_id = "rId2"
                content_types = add_chart_content_type(content_types, next_chart_no)
                next_chart_no += 1
            new_entries[f"ppt/slides/slide{current_slide_no}.xml"] = slide_xml(
                slide,
                current_slide_no,
                chart_rel_id=chart_rel_id,
            )
            new_entries[f"ppt/slides/_rels/slide{current_slide_no}.xml.rels"] = slide_rels(chart_target=chart_target)
            rels = rels.replace(
                "</Relationships>",
                f'<Relationship Id="rId{current_rel_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{current_slide_no}.xml"/></Relationships>',
            )
            presentation = presentation.replace(
                "</p:sldIdLst>",
                f'<p:sldId id="{current_slide_id}" r:id="rId{current_rel_id}"/></p:sldIdLst>',
            )
            content_types = add_content_type(content_types, current_slide_no)

        with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
            for item in zin.infolist():
                if item.filename == "ppt/presentation.xml":
                    zout.writestr(item.filename, presentation)
                elif item.filename == "ppt/_rels/presentation.xml.rels":
                    zout.writestr(item.filename, rels)
                elif item.filename == "[Content_Types].xml":
                    zout.writestr(item.filename, content_types)
                else:
                    zout.writestr(item, zin.read(item.filename))
            for name, data in new_entries.items():
                zout.writestr(name, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, help="Existing .pptx path")
    parser.add_argument("--spec", required=True, help="JSON spec with slides to append")
    parser.add_argument("--out", required=True, help="Output .pptx path")
    ns = parser.parse_args()
    append_pptx(Path(ns.input), json.loads(Path(ns.spec).read_text(encoding="utf-8")), Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
