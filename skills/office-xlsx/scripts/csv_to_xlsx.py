#!/usr/bin/env python3
"""Convert one or more CSV/TSV files into an editable XLSX workbook."""

from __future__ import annotations

import argparse
import csv
from pathlib import Path

from build_xlsx import write_xlsx


def infer_value(value: str):
    value = value.strip()
    if value == "":
        return None
    lower = value.lower()
    if lower in {"true", "false"}:
        return lower == "true"
    if value.startswith("="):
        return value
    try:
        if "." not in value and "e" not in lower:
            return int(value)
        return float(value)
    except ValueError:
        return value


def choose_dialect(path: Path, delimiter: str) -> csv.Dialect:
    sample = path.read_text(encoding="utf-8-sig")[:4096]
    if delimiter == "auto":
        if path.suffix.lower() == ".tsv":
            delimiter = "tab"
        else:
            try:
                return csv.Sniffer().sniff(sample, delimiters=",\t;")
            except csv.Error:
                delimiter = "comma"
    delim = {"comma": ",", "tab": "\t", "semicolon": ";"}.get(delimiter, delimiter)
    class Dialect(csv.excel):
        pass
    Dialect.delimiter = delim
    return Dialect


def read_table(path: Path, delimiter: str, infer_types: bool) -> list[list[object]]:
    dialect = choose_dialect(path, delimiter)
    with path.open("r", encoding="utf-8-sig", newline="") as handle:
        rows = list(csv.reader(handle, dialect))
    if infer_types:
        return [[infer_value(value) for value in row] for row in rows]
    return rows


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", action="append", required=True, help="CSV/TSV input path")
    parser.add_argument("--sheet", action="append", default=[], help="Optional sheet name")
    parser.add_argument("--out", required=True, help="Output .xlsx path")
    parser.add_argument("--delimiter", default="auto", help="auto, comma, tab, semicolon, or a literal delimiter")
    parser.add_argument("--no-infer-types", action="store_true")
    ns = parser.parse_args()

    sheets = []
    for idx, raw in enumerate(ns.input):
        path = Path(raw)
        name = ns.sheet[idx] if idx < len(ns.sheet) else path.stem
        sheets.append(
            {
                "name": name,
                "rows": read_table(path, ns.delimiter, not ns.no_infer_types),
                "freeze_top_row": True,
                "autofilter": True,
            }
        )

    write_xlsx({"title": Path(ns.out).stem, "sheets": sheets}, Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
