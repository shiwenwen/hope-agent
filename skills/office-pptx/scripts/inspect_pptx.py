#!/usr/bin/env python3
"""Inspect and verify PPTX packages."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


REQUIRED = {
    "[Content_Types].xml",
    "_rels/.rels",
    "ppt/presentation.xml",
    "ppt/_rels/presentation.xml.rels",
    "ppt/slideMasters/slideMaster1.xml",
    "ppt/slideLayouts/slideLayout1.xml",
    "ppt/theme/theme1.xml",
}


def inspect(path: Path) -> dict:
    with zipfile.ZipFile(path) as zf:
        names = set(zf.namelist())
        missing = sorted(REQUIRED - names)
        slides = sorted(n for n in names if re.fullmatch(r"ppt/slides/slide\d+\.xml", n))
        media = sorted(n for n in names if n.startswith("ppt/media/"))
        charts = sorted(n for n in names if re.fullmatch(r"ppt/charts/chart\d+\.xml", n))
        preview = []
        shape_count = 0
        picture_count = 0
        connector_count = 0
        graphic_frame_count = 0
        for slide in slides:
            root = ET.fromstring(zf.read(slide))
            texts = []
            for node in root.iter():
                if node.tag.endswith("}t") and node.text:
                    texts.append(node.text)
                elif node.tag.endswith("}sp"):
                    shape_count += 1
                elif node.tag.endswith("}pic"):
                    picture_count += 1
                elif node.tag.endswith("}cxnSp"):
                    connector_count += 1
                elif node.tag.endswith("}graphicFrame"):
                    graphic_frame_count += 1
            preview.append({"slide": slide, "text": texts})
        return {
            "path": str(path),
            "valid_package": not missing and bool(slides),
            "missing": missing,
            "slide_count": len(slides),
            "media_count": len(media),
            "chart_count": len(charts),
            "shape_count": shape_count,
            "picture_count": picture_count,
            "connector_count": connector_count,
            "graphic_frame_count": graphic_frame_count,
            "preview": preview,
        }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("path")
    parser.add_argument("--verify", action="store_true")
    ns = parser.parse_args()
    result = inspect(Path(ns.path))
    print(json.dumps(result, ensure_ascii=False, indent=2))
    if ns.verify and not result["valid_package"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
