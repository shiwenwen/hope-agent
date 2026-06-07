#!/usr/bin/env python3
"""Evaluate common XLSX formulas and optionally write cached values."""

from __future__ import annotations

import argparse
import ast
import json
import operator
import re
import zipfile
from dataclasses import dataclass
from pathlib import Path
from xml.etree import ElementTree as ET


MAIN_NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"
OFFICE_REL_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"

ET.register_namespace("", MAIN_NS)


def q(name: str) -> str:
    return f"{{{MAIN_NS}}}{name}"


@dataclass
class Cell:
    ref: str
    value: object | None
    formula: str | None
    element: ET.Element


def col_index(col: str) -> int:
    value = 0
    for char in col.upper():
        value = value * 26 + (ord(char) - 64)
    return value


def col_name(idx: int) -> str:
    out = ""
    while idx:
        idx, rem = divmod(idx - 1, 26)
        out = chr(65 + rem) + out
    return out


def split_ref(ref: str) -> tuple[int, int]:
    match = re.fullmatch(r"\$?([A-Za-z]+)\$?([0-9]+)", ref)
    if not match:
        raise ValueError(ref)
    return col_index(match.group(1)), int(match.group(2))


def expand_range(ref: str) -> list[str]:
    start, end = ref.split(":", 1)
    c1, r1 = split_ref(start)
    c2, r2 = split_ref(end)
    refs = []
    for row in range(min(r1, r2), max(r1, r2) + 1):
        for col in range(min(c1, c2), max(c1, c2) + 1):
            refs.append(f"{col_name(col)}{row}")
    return refs


def split_args(args: str) -> list[str]:
    result = []
    depth = 0
    quote = False
    current = []
    for char in args:
        if char == '"':
            quote = not quote
        elif not quote and char == "(":
            depth += 1
        elif not quote and char == ")":
            depth -= 1
        if char == "," and depth == 0 and not quote:
            result.append("".join(current).strip())
            current = []
        else:
            current.append(char)
    result.append("".join(current).strip())
    return result


def local_name(path: str) -> str:
    return Path(path).name


def workbook_targets(zf: zipfile.ZipFile) -> dict[str, str]:
    workbook = ET.fromstring(zf.read("xl/workbook.xml"))
    rels = ET.fromstring(zf.read("xl/_rels/workbook.xml.rels"))
    targets = {rel.attrib["Id"]: rel.attrib["Target"] for rel in rels if rel.attrib.get("Id")}
    result = {}
    for sheet in workbook.findall(f".//{q('sheet')}"):
        rel_id = sheet.attrib.get(f"{{{OFFICE_REL_NS}}}id")
        name = sheet.attrib.get("name")
        if not rel_id or not name:
            continue
        target = targets.get(rel_id, "")
        if not target.startswith("xl/"):
            target = "xl/" + target.lstrip("/")
        result[name] = target
    return result


def read_cell(cell: ET.Element) -> Cell:
    ref = cell.attrib.get("r", "")
    formula_node = cell.find(q("f"))
    formula = formula_node.text if formula_node is not None else None
    value: object | None = None
    if cell.attrib.get("t") == "inlineStr":
        texts = [node.text or "" for node in cell.iter() if node.tag.endswith("}t")]
        value = "".join(texts)
    else:
        value_node = cell.find(q("v"))
        if value_node is not None and value_node.text is not None:
            raw = value_node.text
            try:
                value = float(raw) if "." in raw else int(raw)
            except ValueError:
                value = raw
    return Cell(ref=ref, value=value, formula=formula, element=cell)


class FormulaEngine:
    def __init__(self, cells: dict[str, dict[str, Cell]]):
        self.cells = cells
        self.stack: set[tuple[str, str]] = set()

    def cell_value(self, sheet: str, ref: str) -> float:
        ref = ref.replace("$", "").upper()
        key = (sheet, ref)
        if key in self.stack:
            raise ValueError(f"circular reference: {sheet}!{ref}")
        cell = self.cells.get(sheet, {}).get(ref)
        if not cell:
            return 0.0
        if cell.formula:
            self.stack.add(key)
            try:
                value = self.evaluate(sheet, cell.formula)
            finally:
                self.stack.remove(key)
            return float(value)
        if isinstance(cell.value, (int, float)):
            return float(cell.value)
        return 0.0

    def range_values(self, sheet: str, ref: str) -> list[float]:
        return [self.cell_value(sheet, cell_ref) for cell_ref in expand_range(ref.replace("$", ""))]

    def evaluate(self, sheet: str, formula: str) -> float:
        expr = formula.strip().lstrip("=")
        expr = expr.replace("^", "**")

        def replace_if(match: re.Match[str]) -> str:
            args = split_args(match.group(1))
            if len(args) != 3:
                raise ValueError("IF requires three arguments")
            chosen = args[1] if bool(self.evaluate(sheet, args[0])) else args[2]
            return str(self.evaluate(sheet, chosen))

        expr = re.sub(r"\bIF\(([^()]*(?:\([^()]*\)[^()]*)*)\)", replace_if, expr, flags=re.I)

        def replace_function(match: re.Match[str]) -> str:
            name = match.group(1).upper()
            args = split_args(match.group(2))
            values: list[float] = []
            if name in {"ROUND", "ABS"}:
                if name == "ABS":
                    if len(args) != 1:
                        raise ValueError("ABS requires one argument")
                    return str(abs(float(self.evaluate(sheet, args[0]))))
                if len(args) not in {1, 2}:
                    raise ValueError("ROUND requires one or two arguments")
                digits = int(self.evaluate(sheet, args[1])) if len(args) == 2 else 0
                return str(round(float(self.evaluate(sheet, args[0])), digits))
            for item in args:
                target_sheet = sheet
                if "!" in item:
                    target_sheet, item = normalize_sheet_ref(item)
                if ":" in item:
                    values.extend(self.range_values(target_sheet, item))
                else:
                    values.append(self.cell_value(target_sheet, item))
            if name == "SUM":
                return str(sum(values))
            if name == "AVERAGE":
                return str(sum(values) / len(values) if values else 0)
            if name == "MIN":
                return str(min(values) if values else 0)
            if name == "MAX":
                return str(max(values) if values else 0)
            if name == "COUNT":
                return str(len([v for v in values if isinstance(v, (int, float))]))
            if name == "MEDIAN":
                ordered = sorted(values)
                if not ordered:
                    return "0"
                mid = len(ordered) // 2
                if len(ordered) % 2:
                    return str(ordered[mid])
                return str((ordered[mid - 1] + ordered[mid]) / 2)
            raise ValueError(f"unsupported function: {name}")

        expr = re.sub(r"\b(SUM|AVERAGE|MIN|MAX|COUNT|MEDIAN|ROUND|ABS)\(([^()]*)\)", replace_function, expr, flags=re.I)

        def replace_ref(match: re.Match[str]) -> str:
            raw = match.group(0)
            target_sheet = sheet
            ref = raw
            if "!" in raw:
                target_sheet, ref = normalize_sheet_ref(raw)
            return str(self.cell_value(target_sheet, ref))

        expr = re.sub(
            r"(?:'[^']+'|[A-Za-z_][\w ]*)!\$?[A-Za-z]{1,3}\$?[0-9]+|\$?[A-Za-z]{1,3}\$?[0-9]+",
            replace_ref,
            expr,
        )
        return safe_eval(expr)


def normalize_sheet_ref(raw: str) -> tuple[str, str]:
    sheet, ref = raw.split("!", 1)
    sheet = sheet.strip("'")
    return sheet, ref


def safe_eval(expr: str) -> float | bool:
    allowed = {
        ast.Add: operator.add,
        ast.Sub: operator.sub,
        ast.Mult: operator.mul,
        ast.Div: operator.truediv,
        ast.Pow: operator.pow,
        ast.USub: operator.neg,
        ast.UAdd: operator.pos,
        ast.Eq: operator.eq,
        ast.NotEq: operator.ne,
        ast.Lt: operator.lt,
        ast.LtE: operator.le,
        ast.Gt: operator.gt,
        ast.GtE: operator.ge,
    }

    def visit(node: ast.AST) -> float | bool:
        if isinstance(node, ast.Expression):
            return visit(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, (int, float, bool)):
            return node.value if isinstance(node.value, bool) else float(node.value)
        if isinstance(node, ast.BinOp) and type(node.op) in allowed:
            return allowed[type(node.op)](visit(node.left), visit(node.right))
        if isinstance(node, ast.UnaryOp) and type(node.op) in allowed:
            return allowed[type(node.op)](visit(node.operand))
        if isinstance(node, ast.Compare):
            left = visit(node.left)
            for op, comparator in zip(node.ops, node.comparators):
                right = visit(comparator)
                if type(op) not in allowed or not allowed[type(op)](left, right):
                    return False
                left = right
            return True
        raise ValueError(f"unsupported expression: {expr}")

    return visit(ast.parse(expr, mode="eval"))


def audit(src: Path, write_cache: Path | None = None) -> dict:
    with zipfile.ZipFile(src) as zin:
        targets = workbook_targets(zin)
        roots: dict[str, ET.Element] = {}
        cells: dict[str, dict[str, Cell]] = {}
        for sheet, target in targets.items():
            root = ET.fromstring(zin.read(target))
            roots[target] = root
            sheet_cells = {}
            for cell in root.iter(q("c")):
                parsed = read_cell(cell)
                if parsed.ref:
                    sheet_cells[parsed.ref.upper()] = parsed
            cells[sheet] = sheet_cells

        engine = FormulaEngine(cells)
        evaluated = []
        unsupported = []
        errors = []
        for sheet, sheet_cells in cells.items():
            for ref, cell in sheet_cells.items():
                if not cell.formula:
                    continue
                try:
                    value = engine.evaluate(sheet, cell.formula)
                    evaluated.append({"sheet": sheet, "cell": ref, "formula": "=" + cell.formula, "value": value})
                    if write_cache:
                        cache_node = cell.element.find(q("v"))
                        if cache_node is None:
                            cache_node = ET.SubElement(cell.element, q("v"))
                        cache_node.text = format_number(value)
                except ValueError as exc:
                    unsupported.append({"sheet": sheet, "cell": ref, "formula": "=" + cell.formula, "reason": str(exc)})
                except Exception as exc:
                    errors.append({"sheet": sheet, "cell": ref, "formula": "=" + cell.formula, "reason": str(exc)})

        if write_cache:
            write_cache.parent.mkdir(parents=True, exist_ok=True)
            with zipfile.ZipFile(write_cache, "w", compression=zipfile.ZIP_DEFLATED) as zout:
                for item in zin.infolist():
                    if item.filename in roots:
                        zout.writestr(item.filename, ET.tostring(roots[item.filename], encoding="utf-8", xml_declaration=True))
                    else:
                        zout.writestr(item, zin.read(item.filename))

    return {
        "path": str(src),
        "cache_written": str(write_cache) if write_cache else None,
        "evaluated_count": len(evaluated),
        "unsupported_count": len(unsupported),
        "error_count": len(errors),
        "evaluated": evaluated[:200],
        "unsupported": unsupported[:200],
        "errors": errors[:200],
        "passed": not errors,
    }


def format_number(value: float) -> str:
    if value == int(value):
        return str(int(value))
    return f"{value:.12g}"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--write-cache")
    parser.add_argument("--fail-on-unsupported", action="store_true")
    ns = parser.parse_args()
    result = audit(Path(ns.input), Path(ns.write_cache) if ns.write_cache else None)
    print(json.dumps(result, ensure_ascii=False, indent=2))
    if not result["passed"] or (ns.fail_on_unsupported and result["unsupported_count"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
