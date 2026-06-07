#!/usr/bin/env python3
"""Extract DOCX comments as JSON."""

from __future__ import annotations

import argparse
import json
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


def extract(path: Path) -> list[dict]:
    with zipfile.ZipFile(path) as zf:
        if "word/comments.xml" not in zf.namelist():
            return []
        root = ET.fromstring(zf.read("word/comments.xml"))
    comments = []
    for node in root:
        text = "".join(child.text or "" for child in node.iter() if child.tag.endswith("}t"))
        comments.append(
            {
                "id": node.attrib.get("{http://schemas.openxmlformats.org/wordprocessingml/2006/main}id"),
                "author": node.attrib.get("{http://schemas.openxmlformats.org/wordprocessingml/2006/main}author"),
                "date": node.attrib.get("{http://schemas.openxmlformats.org/wordprocessingml/2006/main}date"),
                "text": text,
            }
        )
    return comments


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    ns = parser.parse_args()
    print(json.dumps(extract(Path(ns.input)), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
