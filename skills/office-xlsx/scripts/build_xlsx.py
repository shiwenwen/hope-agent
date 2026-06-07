#!/usr/bin/env python3
"""Build an editable XLSX workbook from JSON using Python stdlib only."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from xml.sax.saxutils import escape


def x(value: object) -> str:
    return escape("" if value is None else str(value))


def col_name(idx: int) -> str:
    name = ""
    while idx:
        idx, rem = divmod(idx - 1, 26)
        name = chr(65 + rem) + name
    return name


def sanitize_sheet_name(name: str, fallback: str) -> str:
    cleaned = re.sub(r"[\[\]\:\*\?\/\\]", "_", name or fallback).strip("'")[:31]
    return cleaned or fallback


def sheet_name(sheet: dict, idx: int) -> str:
    return sanitize_sheet_name(str(sheet.get("name", "")), f"Sheet{idx}")


def has_charts(sheet: dict) -> bool:
    return bool(sheet.get("charts"))


def parse_cell_ref(ref: str) -> tuple[int, int]:
    match = re.fullmatch(r"([A-Za-z]+)([0-9]+)", ref.strip())
    if not match:
        return 3, 2
    col = 0
    for char in match.group(1).upper():
        col = col * 26 + (ord(char) - 64)
    return max(col - 1, 0), max(int(match.group(2)) - 1, 0)


def range_formula(ref: str, name: str) -> str:
    if "!" in ref:
        return ref
    quoted = name.replace("'", "''")
    return f"'{quoted}'!{ref}"


def column_format_style(fmt: object) -> int:
    name = str(fmt or "").lower()
    return {
        "header": 1,
        "currency": 2,
        "money": 2,
        "percent": 3,
        "percentage": 3,
        "date": 4,
        "number": 5,
        "integer": 5,
        "text": 6,
    }.get(name, 0)


def cell_xml(
    row_idx: int,
    col_idx: int,
    value: object,
    formula: str | None = None,
    style: int = 0,
) -> str:
    ref = f"{col_name(col_idx)}{row_idx}"
    style_attr = f' s="{style}"' if style else ""
    if formula:
        return f'<c r="{ref}"{style_attr}><f>{x(formula.lstrip("="))}</f></c>'
    if value is None:
        return f'<c r="{ref}"{style_attr}/>'
    if isinstance(value, bool):
        return f'<c r="{ref}"{style_attr} t="b"><v>{1 if value else 0}</v></c>'
    if isinstance(value, (int, float)):
        return f'<c r="{ref}"{style_attr}><v>{value}</v></c>'
    text = str(value)
    if text.startswith("="):
        return f'<c r="{ref}"{style_attr}><f>{x(text[1:])}</f></c>'
    return f'<c r="{ref}"{style_attr} t="inlineStr"><is><t>{x(text)}</t></is></c>'


def local_table_rel_ids(sheet: dict) -> list[str]:
    start = 2 if has_charts(sheet) else 1
    return [f"rId{start + idx}" for idx, _table in enumerate(sheet.get("tables") or [])]


def data_validations_xml(sheet: dict) -> str:
    validations = sheet.get("data_validations") or sheet.get("validations") or []
    if not validations:
        return ""
    parts = []
    for item in validations:
        ref = str(item.get("range") or item.get("ref") or "")
        if not ref:
            continue
        kind = str(item.get("type") or "list")
        allow_blank = "1" if item.get("allow_blank", True) else "0"
        attrs = [
            f'type="{x(kind)}"',
            f'allowBlank="{allow_blank}"',
            f'sqref="{x(ref)}"',
        ]
        if item.get("operator"):
            attrs.append(f'operator="{x(item["operator"])}"')
        formula1 = item.get("formula1")
        if isinstance(formula1, list):
            formula1 = '"' + ",".join(str(value) for value in formula1) + '"'
        formula2 = item.get("formula2")
        parts.append(
            "<dataValidation "
            + " ".join(attrs)
            + ">"
            + (f"<formula1>{x(formula1)}</formula1>" if formula1 is not None else "")
            + (f"<formula2>{x(formula2)}</formula2>" if formula2 is not None else "")
            + "</dataValidation>"
        )
    if not parts:
        return ""
    return f'<dataValidations count="{len(parts)}">' + "".join(parts) + "</dataValidations>"


def conditional_formats_xml(sheet: dict) -> str:
    formats = sheet.get("conditional_formats") or sheet.get("conditional_formatting") or []
    parts = []
    priority = 1
    for item in formats:
        ref = str(item.get("range") or item.get("ref") or "")
        if not ref:
            continue
        kind = str(item.get("type") or "colorScale")
        if kind == "colorScale":
            parts.append(
                f'<conditionalFormatting sqref="{x(ref)}"><cfRule type="colorScale" priority="{priority}">'
                '<colorScale><cfvo type="min"/><cfvo type="percentile" val="50"/><cfvo type="max"/>'
                '<color rgb="FFF8696B"/><color rgb="FFFFEB84"/><color rgb="FF63BE7B"/>'
                "</colorScale></cfRule></conditionalFormatting>"
            )
        elif kind == "cellIs":
            operator = str(item.get("operator") or "greaterThan")
            formula = str(item.get("formula") or item.get("formula1") or "0").lstrip("=")
            dxf_id = {"good": 0, "warning": 1, "bad": 2}.get(str(item.get("style") or "good"), 0)
            parts.append(
                f'<conditionalFormatting sqref="{x(ref)}"><cfRule type="cellIs" dxfId="{dxf_id}" priority="{priority}" operator="{x(operator)}"><formula>{x(formula)}</formula></cfRule></conditionalFormatting>'
            )
        priority += 1
    return "".join(parts)


def sheet_xml(sheet: dict, sheet_idx: int = 1) -> str:
    rows = sheet.get("rows") or []
    formula_by_ref = {
        str(item.get("cell", "")).upper(): str(item.get("formula", "")).lstrip("=")
        for item in sheet.get("formulas") or []
        if item.get("cell") and item.get("formula")
    }
    max_cols = max((len(row) for row in rows), default=1)
    widths = sheet.get("column_widths") or []
    cols_xml = ""
    if widths:
        cols = []
        for idx, width in enumerate(widths, 1):
            cols.append(f'<col min="{idx}" max="{idx}" width="{float(width)}" customWidth="1"/>')
        cols_xml = "<cols>" + "".join(cols) + "</cols>"
    views = ""
    if sheet.get("freeze_top_row", True):
        views = '<sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>'
    row_xml = []
    column_formats = sheet.get("column_formats") or []
    header_style = bool(sheet.get("header_style", True))
    for r_idx, row in enumerate(rows, 1):
        cells = "".join(
            cell_xml(
                r_idx,
                c_idx,
                value,
                formula_by_ref.pop(f"{col_name(c_idx)}{r_idx}".upper(), None),
                1 if header_style and r_idx == 1 else column_format_style(column_formats[c_idx - 1] if c_idx - 1 < len(column_formats) else None),
            )
            for c_idx, value in enumerate(row, 1)
        )
        row_xml.append(f'<row r="{r_idx}">{cells}</row>')
    for ref, formula in sorted(formula_by_ref.items()):
        match = re.fullmatch(r"([A-Z]+)([0-9]+)", ref)
        if match:
            row_xml.append(f'<row r="{match.group(2)}"><c r="{x(ref)}"><f>{x(formula)}</f></c></row>')
    auto_filter = ""
    if rows and sheet.get("autofilter", True):
        auto_filter = f'<autoFilter ref="A1:{col_name(max_cols)}{len(rows)}"/>'
    drawing = '<drawing r:id="rId1"/>' if has_charts(sheet) else ""
    table_parts = ""
    table_rel_ids = local_table_rel_ids(sheet)
    if table_rel_ids:
        table_parts = (
            f'<tableParts count="{len(table_rel_ids)}">'
            + "".join(f'<tablePart r:id="{rid}"/>' for rid in table_rel_ids)
            + "</tableParts>"
        )
    ns_r = ' xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"{ns_r}>'
        + views
        + cols_xml
        + "<sheetData>"
        + "".join(row_xml)
        + "</sheetData>"
        + auto_filter
        + conditional_formats_xml(sheet)
        + data_validations_xml(sheet)
        + drawing
        + table_parts
        + "</worksheet>"
    )


def workbook_xml(sheets: list[dict]) -> str:
    entries = []
    for idx, sheet in enumerate(sheets, 1):
        entries.append(f'<sheet name="{x(sheet_name(sheet, idx))}" sheetId="{idx}" r:id="rId{idx}"/>')
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        "<sheets>"
        + "".join(entries)
        + '</sheets><calcPr calcMode="auto" fullCalcOnLoad="1" forceFullCalc="1"/></workbook>'
    )


def workbook_rels(sheets: list[dict]) -> str:
    rels = []
    for idx, _sheet in enumerate(sheets, 1):
        rels.append(
            f'<Relationship Id="rId{idx}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{idx}.xml"/>'
        )
    rels.append(
        f'<Relationship Id="rId{len(sheets)+1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + "".join(rels)
        + "</Relationships>"
    )


def core_xml(title: str) -> str:
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>{x(title)}</dc:title><dc:creator>Hope Agent</dc:creator>
  <dcterms:created xsi:type="dcterms:W3CDTF">{now}</dcterms:created>
  <dcterms:modified xsi:type="dcterms:W3CDTF">{now}</dcterms:modified>
</cp:coreProperties>'''


def styles_xml() -> str:
    return '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="2"><numFmt numFmtId="164" formatCode="$#,##0.00"/><numFmt numFmtId="165" formatCode="@"/></numFmts>
  <fonts count="2"><font><sz val="11"/><name val="Aptos"/></font><font><b/><sz val="11"/><name val="Aptos"/></font></fonts>
  <fills count="4"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFEAF2F8"/><bgColor indexed="64"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FFE2F0D9"/><bgColor indexed="64"/></patternFill></fill></fills>
  <borders count="2"><border/><border><left style="thin"><color rgb="FFD9E2F3"/></left><right style="thin"><color rgb="FFD9E2F3"/></right><top style="thin"><color rgb="FFD9E2F3"/></top><bottom style="thin"><color rgb="FFD9E2F3"/></bottom></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="7">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0" applyFont="1" applyFill="1" applyBorder="1"/>
    <xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="10" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="4" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="165" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
  </cellXfs>
  <dxfs count="3">
    <dxf><fill><patternFill patternType="solid"><fgColor rgb="FFE2F0D9"/></patternFill></fill></dxf>
    <dxf><fill><patternFill patternType="solid"><fgColor rgb="FFFFF2CC"/></patternFill></fill></dxf>
    <dxf><fill><patternFill patternType="solid"><fgColor rgb="FFFCE4D6"/></patternFill></fill></dxf>
  </dxfs>
  <tableStyles count="0" defaultTableStyle="TableStyleMedium2" defaultPivotStyle="PivotStyleLight16"/>
</styleSheet>'''


def chart_plan(sheets: list[dict]) -> list[tuple[int, int, dict]]:
    plan = []
    chart_idx = 1
    for sheet_idx, sheet in enumerate(sheets, 1):
        for chart in sheet.get("charts") or []:
            plan.append((sheet_idx, chart_idx, chart))
            chart_idx += 1
    return plan


def table_plan(sheets: list[dict]) -> list[tuple[int, int, dict]]:
    plan = []
    table_idx = 1
    for sheet_idx, sheet in enumerate(sheets, 1):
        for table in sheet.get("tables") or []:
            plan.append((sheet_idx, table_idx, table))
            table_idx += 1
    return plan


def content_types(sheets: list[dict]) -> str:
    charts = chart_plan(sheets)
    tables = table_plan(sheets)
    overrides = [
        '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>',
        '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>',
        '<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>',
        '<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>',
    ]
    for idx, _sheet in enumerate(sheets, 1):
        overrides.append(
            f'<Override PartName="/xl/worksheets/sheet{idx}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        )
        if has_charts(_sheet):
            overrides.append(
                f'<Override PartName="/xl/drawings/drawing{idx}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>'
            )
    for _sheet_idx, chart_idx, _chart in charts:
        overrides.append(
            f'<Override PartName="/xl/charts/chart{chart_idx}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>'
        )
    for _sheet_idx, table_idx, _table in tables:
        overrides.append(
            f'<Override PartName="/xl/tables/table{table_idx}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>'
        )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        + "".join(overrides)
        + "</Types>"
    )


def sheet_rels(sheet_idx: int, sheet: dict, table_ids: list[int]) -> str:
    rels = []
    rel_id = 1
    if has_charts(sheet):
        rels.append(
            f'<Relationship Id="rId{rel_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing{sheet_idx}.xml"/>'
        )
        rel_id += 1
    for table_idx in table_ids:
        rels.append(
            f'<Relationship Id="rId{rel_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table{table_idx}.xml"/>'
        )
        rel_id += 1
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + "".join(rels)
        + "</Relationships>"
    )


def drawing_xml(sheet_idx: int, charts: list[tuple[int, dict]]) -> str:
    anchors = []
    for local_idx, (_chart_idx, chart) in enumerate(charts, 1):
        anchor_ref = str(chart.get("anchor") or f"D{2 + (local_idx - 1) * 16}")
        col, row = parse_cell_ref(anchor_ref)
        anchors.append(
            f'''<xdr:twoCellAnchor>
  <xdr:from><xdr:col>{col}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>{col + 8}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{row + 15}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
  <xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="{local_idx + 1}" name="Chart {local_idx}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId{local_idx}"/></a:graphicData></a:graphic></xdr:graphicFrame>
  <xdr:clientData/>
</xdr:twoCellAnchor>'''
        )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">'
        + "".join(anchors)
        + "</xdr:wsDr>"
    )


def drawing_rels(charts: list[tuple[int, dict]]) -> str:
    rels = []
    for local_idx, (chart_idx, _chart) in enumerate(charts, 1):
        rels.append(
            f'<Relationship Id="rId{local_idx}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart{chart_idx}.xml"/>'
        )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + "".join(rels)
        + "</Relationships>"
    )


def chart_xml(chart: dict, sheet: dict, sheet_idx: int) -> str:
    rows = sheet.get("rows") or []
    max_row = max(len(rows), 2)
    name = sheet_name(sheet, sheet_idx)
    title = str(chart.get("title") or "Chart")
    categories = range_formula(str(chart.get("categories") or f"$A$2:$A${max_row}"), name)
    values = range_formula(str(chart.get("values") or f"$B$2:$B${max_row}"), name)
    chart_kind = str(chart.get("type") or "bar").lower()
    series_xml = f'''<c:ser><c:idx val="0"/><c:order val="0"/>
          <c:cat><c:strRef><c:f>{x(categories)}</c:f></c:strRef></c:cat>
          <c:val><c:numRef><c:f>{x(values)}</c:f></c:numRef></c:val>
        </c:ser>'''
    if chart_kind in {"line", "line_chart"}:
        plot_xml = f'''<c:lineChart><c:grouping val="standard"/>
        {series_xml}
        <c:axId val="48650112"/><c:axId val="48672768"/>
      </c:lineChart>
      <c:catAx><c:axId val="48650112"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:tickLblPos val="nextTo"/><c:crossAx val="48672768"/><c:crosses val="autoZero"/></c:catAx>
      <c:valAx><c:axId val="48672768"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="l"/><c:majorGridlines/><c:tickLblPos val="nextTo"/><c:crossAx val="48650112"/><c:crosses val="autoZero"/></c:valAx>'''
    elif chart_kind in {"pie", "pie_chart"}:
        plot_xml = f'''<c:pieChart>
        {series_xml}
        <c:firstSliceAng val="0"/>
      </c:pieChart>'''
    else:
        direction = "bar" if chart_kind in {"bar", "horizontal_bar"} else "col"
        plot_xml = f'''<c:barChart><c:barDir val="{direction}"/><c:grouping val="clustered"/>
        {series_xml}
        <c:axId val="48650112"/><c:axId val="48672768"/>
      </c:barChart>
      <c:catAx><c:axId val="48650112"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:tickLblPos val="nextTo"/><c:crossAx val="48672768"/><c:crosses val="autoZero"/></c:catAx>
      <c:valAx><c:axId val="48672768"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="l"/><c:majorGridlines/><c:tickLblPos val="nextTo"/><c:crossAx val="48650112"/><c:crosses val="autoZero"/></c:valAx>'''
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <c:chart>
    <c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{x(title)}</a:t></a:r></a:p></c:rich></c:tx><c:layout/></c:title>
    <c:plotArea><c:layout/>
      {plot_xml}
    </c:plotArea>
    <c:legend><c:legendPos val="r"/><c:layout/></c:legend>
    <c:plotVisOnly val="1"/>
  </c:chart>
</c:chartSpace>'''


def sanitize_table_name(name: str, fallback: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9_]", "_", name or fallback)
    if not cleaned or cleaned[0].isdigit():
        cleaned = f"Table_{cleaned or fallback}"
    return cleaned[:255]


def table_ref(table: dict, sheet: dict) -> str:
    if table.get("ref"):
        return str(table["ref"])
    rows = sheet.get("rows") or []
    max_rows = max(len(rows), 1)
    max_cols = max((len(row) for row in rows), default=1)
    return f"A1:{col_name(max_cols)}{max_rows}"


def table_columns(table: dict, sheet: dict) -> list[str]:
    explicit = table.get("columns")
    if explicit:
        return [str(value) for value in explicit]
    rows = sheet.get("rows") or []
    if rows:
        return [str(value or f"Column{idx}") for idx, value in enumerate(rows[0], 1)]
    return ["Column1"]


def table_xml(table: dict, sheet: dict, table_idx: int) -> str:
    ref = table_ref(table, sheet)
    name = sanitize_table_name(str(table.get("name") or f"Table{table_idx}"), f"Table{table_idx}")
    columns = table_columns(table, sheet)
    style = str(table.get("style") or "TableStyleMedium2")
    column_xml = "".join(
        f'<tableColumn id="{idx}" name="{x(name)}"/>' for idx, name in enumerate(columns, 1)
    )
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="{table_idx}" name="{x(name)}" displayName="{x(name)}" ref="{x(ref)}" totalsRowShown="0">
  <autoFilter ref="{x(ref)}"/>
  <tableColumns count="{len(columns)}">{column_xml}</tableColumns>
  <tableStyleInfo name="{x(style)}" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/>
</table>'''


def write_xlsx(spec: dict, out: Path) -> None:
    sheets = spec.get("sheets") or [{"name": "Sheet1", "rows": spec.get("rows", [])}]
    charts = chart_plan(sheets)
    tables = table_plan(sheets)
    out.parent.mkdir(parents=True, exist_ok=True)
    entries = {
        "[Content_Types].xml": content_types(sheets),
        "_rels/.rels": '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>''',
        "docProps/app.xml": '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Hope Agent</Application></Properties>''',
        "docProps/core.xml": core_xml(str(spec.get("title") or out.stem)),
        "xl/workbook.xml": workbook_xml(sheets),
        "xl/_rels/workbook.xml.rels": workbook_rels(sheets),
        "xl/styles.xml": styles_xml(),
    }
    for idx, sheet in enumerate(sheets, 1):
        entries[f"xl/worksheets/sheet{idx}.xml"] = sheet_xml(sheet, idx)
        sheet_tables = [table_idx for s_idx, table_idx, _table in tables if s_idx == idx]
        if has_charts(sheet) or sheet_tables:
            sheet_charts = [(chart_idx, chart) for s_idx, chart_idx, chart in charts if s_idx == idx]
            entries[f"xl/worksheets/_rels/sheet{idx}.xml.rels"] = sheet_rels(idx, sheet, sheet_tables)
            if has_charts(sheet):
                entries[f"xl/drawings/drawing{idx}.xml"] = drawing_xml(idx, sheet_charts)
                entries[f"xl/drawings/_rels/drawing{idx}.xml.rels"] = drawing_rels(sheet_charts)
    for sheet_idx, chart_idx, chart in charts:
        entries[f"xl/charts/chart{chart_idx}.xml"] = chart_xml(chart, sheets[sheet_idx - 1], sheet_idx)
    for sheet_idx, table_idx, table in tables:
        entries[f"xl/tables/table{table_idx}.xml"] = table_xml(table, sheets[sheet_idx - 1], table_idx)
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries.items():
            zf.writestr(name, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--spec", required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    write_xlsx(json.loads(Path(ns.spec).read_text(encoding="utf-8")), Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
