#!/usr/bin/env python3
"""Patch an existing XLSX workbook with simple, deterministic edits.

Supported actions:
- append_rows: append rows to an existing sheet
- set_cell: set or replace a single cell value/formula
"""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


MAIN_NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"

ET.register_namespace("", MAIN_NS)


def q(name: str) -> str:
    return f"{{{MAIN_NS}}}{name}"


def col_name(idx: int) -> str:
    name = ""
    while idx:
        idx, rem = divmod(idx - 1, 26)
        name = chr(65 + rem) + name
    return name


def col_index(ref: str) -> int:
    match = re.match(r"([A-Z]+)", ref.upper())
    if not match:
        return 0
    value = 0
    for char in match.group(1):
        value = value * 26 + (ord(char) - 64)
    return value


def parse_cell_ref(ref: str) -> tuple[int, int]:
    match = re.fullmatch(r"([A-Za-z]+)([0-9]+)", ref.strip())
    if not match:
        raise ValueError(f"Invalid cell reference: {ref}")
    return col_index(match.group(1)), int(match.group(2))


def make_cell(row_idx: int, col_idx: int, value: object) -> ET.Element:
    cell = ET.Element(q("c"), {"r": f"{col_name(col_idx)}{row_idx}"})
    if value is None:
        return cell
    if isinstance(value, bool):
        cell.set("t", "b")
        ET.SubElement(cell, q("v")).text = "1" if value else "0"
    elif isinstance(value, (int, float)):
        ET.SubElement(cell, q("v")).text = str(value)
    else:
        text = str(value)
        if text.startswith("="):
            ET.SubElement(cell, q("f")).text = text[1:]
        else:
            cell.set("t", "inlineStr")
            inline = ET.SubElement(cell, q("is"))
            ET.SubElement(inline, q("t")).text = text
    return cell


def workbook_sheet_targets(zf: zipfile.ZipFile) -> dict[str, str]:
    workbook = ET.fromstring(zf.read("xl/workbook.xml"))
    rels = ET.fromstring(zf.read("xl/_rels/workbook.xml.rels"))
    rel_targets = {
        rel.attrib["Id"]: rel.attrib["Target"].lstrip("/")
        for rel in rels
        if rel.attrib.get("Id") and rel.attrib.get("Target")
    }
    targets = {}
    for sheet in workbook.findall(f".//{q('sheet')}"):
        name = sheet.attrib.get("name")
        rel_id = sheet.attrib.get("{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id")
        if name and rel_id and rel_id in rel_targets:
            target = rel_targets[rel_id]
            if not target.startswith("xl/"):
                target = f"xl/{target}"
            targets[name] = target
    return targets


def get_sheet_data(root: ET.Element) -> ET.Element:
    sheet_data = root.find(q("sheetData"))
    if sheet_data is None:
        sheet_data = ET.SubElement(root, q("sheetData"))
    return sheet_data


def find_row(sheet_data: ET.Element, row_idx: int) -> ET.Element:
    for row in sheet_data.findall(q("row")):
        if int(row.attrib.get("r", "0")) == row_idx:
            return row
    row = ET.Element(q("row"), {"r": str(row_idx)})
    inserted = False
    for idx, existing in enumerate(list(sheet_data)):
        if int(existing.attrib.get("r", "0")) > row_idx:
            sheet_data.insert(idx, row)
            inserted = True
            break
    if not inserted:
        sheet_data.append(row)
    return row


def set_cell(root: ET.Element, ref: str, value: object) -> None:
    col_idx, row_idx = parse_cell_ref(ref)
    row = find_row(get_sheet_data(root), row_idx)
    target_ref = f"{col_name(col_idx)}{row_idx}"
    for cell in list(row):
        if cell.attrib.get("r") == target_ref:
            row.remove(cell)
    new_cell = make_cell(row_idx, col_idx, value)
    inserted = False
    for idx, existing in enumerate(list(row)):
        if col_index(existing.attrib.get("r", "")) > col_idx:
            row.insert(idx, new_cell)
            inserted = True
            break
    if not inserted:
        row.append(new_cell)


def append_rows(root: ET.Element, rows: list[list[object]]) -> None:
    sheet_data = get_sheet_data(root)
    last_row = max((int(row.attrib.get("r", "0")) for row in sheet_data.findall(q("row"))), default=0)
    for offset, values in enumerate(rows, 1):
        row_idx = last_row + offset
        row = ET.Element(q("row"), {"r": str(row_idx)})
        for col_idx, value in enumerate(values, 1):
            row.append(make_cell(row_idx, col_idx, value))
        sheet_data.append(row)


def patch_xlsx(src: Path, patch: dict, out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin:
        targets = workbook_sheet_targets(zin)
        sheet_roots: dict[str, ET.Element] = {}
        for action in patch.get("actions", []):
            sheet_name = str(action.get("sheet") or "")
            if sheet_name not in targets:
                raise SystemExit(f"Sheet not found: {sheet_name}")
            target = targets[sheet_name]
            root = sheet_roots.get(target)
            if root is None:
                root = ET.fromstring(zin.read(target))
                sheet_roots[target] = root
            kind = action.get("action")
            if kind == "append_rows":
                append_rows(root, action.get("rows") or [])
            elif kind == "set_cell":
                set_cell(root, str(action.get("cell")), action.get("value"))
            else:
                raise SystemExit(f"Unsupported patch action: {kind}")

        with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
            for item in zin.infolist():
                if item.filename in sheet_roots:
                    zout.writestr(item.filename, ET.tostring(sheet_roots[item.filename], encoding="utf-8", xml_declaration=True))
                else:
                    zout.writestr(item, zin.read(item.filename))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, help="Existing .xlsx path")
    parser.add_argument("--patch", required=True, help="Patch JSON path")
    parser.add_argument("--out", required=True, help="Output .xlsx path")
    ns = parser.parse_args()
    patch_xlsx(Path(ns.input), json.loads(Path(ns.patch).read_text(encoding="utf-8")), Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
