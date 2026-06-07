#!/usr/bin/env python3
"""Extract DOCX tables to CSV files."""

from __future__ import annotations

import argparse
import csv
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


def txt(node: ET.Element) -> str:
    return "".join(child.text or "" for child in node.iter() if child.tag.endswith("}t"))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--out-dir", required=True)
    ns = parser.parse_args()
    out_dir = Path(ns.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(ns.input) as zf:
        root = ET.fromstring(zf.read("word/document.xml"))
    tables = [node for node in root.iter() if node.tag.endswith("}tbl")]
    outputs = []
    for idx, table in enumerate(tables, 1):
        path = out_dir / f"table-{idx}.csv"
        with path.open("w", newline="", encoding="utf-8") as handle:
            writer = csv.writer(handle)
            for row in [n for n in table.iter() if n.tag.endswith("}tr")]:
                cells = [txt(cell) for cell in row if cell.tag.endswith("}tc")]
                writer.writerow(cells)
        outputs.append(str(path))
    print("\n".join(outputs))


if __name__ == "__main__":
    main()
