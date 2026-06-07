#!/usr/bin/env python3
"""Inspect and verify XLSX packages."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


REQUIRED = {"[Content_Types].xml", "_rels/.rels", "xl/workbook.xml", "xl/styles.xml"}


def text_for_node(node: ET.Element) -> str:
    if node.text:
        return node.text
    return "".join(child.text or "" for child in node.iter())


def inspect(path: Path) -> dict:
    with zipfile.ZipFile(path) as zf:
        names = set(zf.namelist())
        missing = sorted(REQUIRED - names)
        sheets = sorted(n for n in names if re.fullmatch(r"xl/worksheets/sheet\d+\.xml", n))
        charts = sorted(n for n in names if re.fullmatch(r"xl/charts/chart\d+\.xml", n))
        tables = sorted(n for n in names if re.fullmatch(r"xl/tables/table\d+\.xml", n))
        previews = []
        formulas = []
        data_validation_count = 0
        conditional_format_count = 0
        for sheet in sheets:
            root = ET.fromstring(zf.read(sheet))
            values = []
            for node in root.iter():
                if node.tag.endswith("}t") and node.text:
                    values.append(node.text)
                elif node.tag.endswith("}v") and node.text:
                    values.append(node.text)
                elif node.tag.endswith("}f") and node.text:
                    formulas.append("=" + node.text)
                elif node.tag.endswith("}dataValidation"):
                    data_validation_count += 1
                elif node.tag.endswith("}conditionalFormatting"):
                    conditional_format_count += 1
            previews.append({"sheet": sheet, "values": values[:80]})
        return {
            "path": str(path),
            "valid_package": not missing and bool(sheets),
            "missing": missing,
            "sheet_count": len(sheets),
            "chart_count": len(charts),
            "table_count": len(tables),
            "data_validation_count": data_validation_count,
            "conditional_format_count": conditional_format_count,
            "formulas": formulas[:80],
            "preview": previews,
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
