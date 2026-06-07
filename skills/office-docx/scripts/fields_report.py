#!/usr/bin/env python3
"""Report Word field instructions from document.xml."""

from __future__ import annotations

import argparse
import json
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


def report(path: Path) -> list[dict]:
    with zipfile.ZipFile(path) as zf:
        root = ET.fromstring(zf.read("word/document.xml"))
    fields = []
    for node in root.iter():
        if node.tag.endswith("}instrText") and node.text:
            fields.append({"instruction": node.text.strip()})
    return fields


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    ns = parser.parse_args()
    print(json.dumps(report(Path(ns.input)), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
