#!/usr/bin/env python3
"""Audit PPTX slide structure, layout bounds, and reference-deck inventory."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


SLIDE_W = 9144000
SLIDE_H = 5143500


def tag_ends(node: ET.Element, suffix: str) -> bool:
    return node.tag.endswith(suffix)


def slide_paths(names: set[str]) -> list[str]:
    return sorted(
        (name for name in names if re.fullmatch(r"ppt/slides/slide\d+\.xml", name)),
        key=lambda value: int(re.search(r"(\d+)", value).group(1)),
    )


def node_texts(root: ET.Element) -> list[str]:
    return [node.text or "" for node in root.iter() if tag_ends(node, "}t") and node.text]


def shape_name(shape: ET.Element) -> str:
    for node in shape.iter():
        if tag_ends(node, "}cNvPr"):
            return node.attrib.get("name", "")
    return ""


def shape_bounds(shape: ET.Element) -> tuple[int, int, int, int] | None:
    off = None
    ext = None
    for node in shape.iter():
        if tag_ends(node, "}off"):
            off = node
        elif tag_ends(node, "}ext"):
            ext = node
        if off is not None and ext is not None:
            break
    if off is None or ext is None:
        return None
    return (
        int(off.attrib.get("x", "0")),
        int(off.attrib.get("y", "0")),
        int(ext.attrib.get("cx", "0")),
        int(ext.attrib.get("cy", "0")),
    )


def inventory(path: Path) -> dict:
    with zipfile.ZipFile(path) as zf:
        names = set(zf.namelist())
        slides = []
        for slide_path in slide_paths(names):
            root = ET.fromstring(zf.read(slide_path))
            texts = node_texts(root)
            shapes = [node for node in root.iter() if tag_ends(node, "}sp")]
            pictures = [node for node in root.iter() if tag_ends(node, "}pic")]
            connectors = [node for node in root.iter() if tag_ends(node, "}cxnSp")]
            graphic_frames = [node for node in root.iter() if tag_ends(node, "}graphicFrame")]
            slide_issues = []
            if not texts and not pictures:
                slide_issues.append({"code": "blank_slide"})
            has_title = any("title" in shape_name(shape).lower() for shape in shapes) or bool(texts)
            if not has_title:
                slide_issues.append({"code": "missing_title"})
            total_chars = sum(len(text) for text in texts)
            if total_chars > 900:
                slide_issues.append({"code": "dense_text", "characters": total_chars})
            out_of_bounds = []
            for idx, shape in enumerate([*shapes, *pictures, *connectors, *graphic_frames], 1):
                bounds = shape_bounds(shape)
                if not bounds:
                    continue
                x, y, cx, cy = bounds
                if x < 0 or y < 0 or x + cx > SLIDE_W or y + cy > SLIDE_H:
                    out_of_bounds.append({"shape": idx, "bounds": bounds})
            if out_of_bounds:
                slide_issues.append({"code": "out_of_bounds", "items": out_of_bounds[:20]})
            slides.append(
                {
                    "slide": slide_path,
                    "title": texts[0] if texts else "",
                    "text_count": len(texts),
                    "character_count": total_chars,
                    "shape_count": len(shapes),
                    "picture_count": len(pictures),
                    "connector_count": len(connectors),
                    "graphic_frame_count": len(graphic_frames),
                    "issues": slide_issues,
                }
            )
        return {
            "path": str(path),
            "slide_count": len(slides),
            "master_count": len([name for name in names if name.startswith("ppt/slideMasters/") and name.endswith(".xml")]),
            "layout_count": len([name for name in names if name.startswith("ppt/slideLayouts/") and name.endswith(".xml")]),
            "theme_count": len([name for name in names if name.startswith("ppt/theme/") and name.endswith(".xml")]),
            "media_count": len([name for name in names if name.startswith("ppt/media/")]),
            "chart_count": len([name for name in names if name.startswith("ppt/charts/") and name.endswith(".xml")]),
            "slides": slides,
        }


def audit(path: Path, reference: Path | None = None) -> dict:
    current = inventory(path)
    issues = []
    for slide in current["slides"]:
        for issue in slide["issues"]:
            issues.append({"slide": slide["slide"], **issue})
    result = {"passed": not issues, "issues": issues, "current": current}
    if reference:
        ref = inventory(reference)
        result["reference"] = ref
        result["template_comparison"] = {
            "reference_slide_count": ref["slide_count"],
            "current_slide_count": current["slide_count"],
            "masters_preserved_or_present": current["master_count"] >= 1,
            "layouts_preserved_or_present": current["layout_count"] >= 1,
            "themes_preserved_or_present": current["theme_count"] >= 1,
        }
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("path")
    parser.add_argument("--reference")
    parser.add_argument("--fail-on-warning", action="store_true")
    ns = parser.parse_args()
    result = audit(Path(ns.path), Path(ns.reference) if ns.reference else None)
    print(json.dumps(result, ensure_ascii=False, indent=2))
    if not result["passed"] or (ns.fail_on_warning and result.get("issues")):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
