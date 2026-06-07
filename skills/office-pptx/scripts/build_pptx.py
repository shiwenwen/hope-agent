#!/usr/bin/env python3
"""Build a basic editable PPTX from JSON using Python stdlib only."""

from __future__ import annotations

import argparse
import json
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from xml.sax.saxutils import escape


P = "http://schemas.openxmlformats.org/presentationml/2006/main"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
C = "http://schemas.openxmlformats.org/drawingml/2006/chart"


def x(value: object) -> str:
    return escape("" if value is None else str(value))


def text_body(lines: list[str], font_size: int = 2400, bullet: bool = False) -> str:
    paras = []
    for line in lines or [""]:
        bu = '<a:buChar char="•"/>' if bullet else "<a:buNone/>"
        paras.append(
            f"<a:p><a:pPr>{bu}</a:pPr><a:r><a:rPr lang=\"en-US\" sz=\"{font_size}\"/>"
            f"<a:t>{x(line)}</a:t></a:r><a:endParaRPr lang=\"en-US\"/></a:p>"
        )
    return (
        "<p:txBody><a:bodyPr wrap=\"square\"/><a:lstStyle/>"
        + "".join(paras)
        + "</p:txBody>"
    )


def shape(
    shape_id: int,
    name: str,
    text: list[str],
    x_pos: int,
    y_pos: int,
    cx: int,
    cy: int,
    font_size: int = 2400,
    bullet: bool = False,
    fill: str | None = None,
    line: str | None = None,
) -> str:
    fill_xml = f'<a:solidFill><a:srgbClr val="{x(fill)}"/></a:solidFill>' if fill else "<a:noFill/>"
    line_xml = f'<a:ln><a:solidFill><a:srgbClr val="{x(line)}"/></a:solidFill></a:ln>' if line else "<a:ln><a:noFill/></a:ln>"
    return f'''<p:sp>
  <p:nvSpPr><p:cNvPr id="{shape_id}" name="{x(name)}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
  <p:spPr><a:xfrm><a:off x="{x_pos}" y="{y_pos}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom>{fill_xml}{line_xml}</p:spPr>
  {text_body(text, font_size, bullet)}
</p:sp>'''


def rect(shape_id: int, name: str, x_pos: int, y_pos: int, cx: int, cy: int, fill: str, line: str | None = None) -> str:
    line_xml = f'<a:ln><a:solidFill><a:srgbClr val="{x(line)}"/></a:solidFill></a:ln>' if line else "<a:ln><a:noFill/></a:ln>"
    return f'''<p:sp>
  <p:nvSpPr><p:cNvPr id="{shape_id}" name="{x(name)}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
  <p:spPr><a:xfrm><a:off x="{x_pos}" y="{y_pos}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="{x(fill)}"/></a:solidFill>{line_xml}</p:spPr>
  <p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody>
</p:sp>'''


def connector(shape_id: int, name: str, x1: int, y1: int, x2: int, y2: int, color: str = "94A3B8") -> str:
    cx = max(abs(x2 - x1), 1)
    cy = max(abs(y2 - y1), 1)
    return f'''<p:cxnSp>
  <p:nvCxnSpPr><p:cNvPr id="{shape_id}" name="{x(name)}"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
  <p:spPr><a:xfrm><a:off x="{min(x1, x2)}" y="{min(y1, y2)}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="line"><a:avLst/></a:prstGeom><a:ln w="19050"><a:solidFill><a:srgbClr val="{x(color)}"/></a:solidFill></a:ln></p:spPr>
</p:cxnSp>'''


def picture(shape_id: int, name: str, rel_id: str, x_pos: int, y_pos: int, cx: int, cy: int) -> str:
    return f'''<p:pic>
  <p:nvPicPr><p:cNvPr id="{shape_id}" name="{x(name)}"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
  <p:blipFill><a:blip r:embed="{rel_id}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>
  <p:spPr><a:xfrm><a:off x="{x_pos}" y="{y_pos}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
</p:pic>'''


def chart_frame(shape_id: int, rel_id: str, x_pos: int, y_pos: int, cx: int, cy: int) -> str:
    return f'''<p:graphicFrame>
  <p:nvGraphicFramePr><p:cNvPr id="{shape_id}" name="Native Chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
  <p:xfrm><a:off x="{x_pos}" y="{y_pos}"/><a:ext cx="{cx}" cy="{cy}"/></p:xfrm>
  <a:graphic><a:graphicData uri="{C}"><c:chart xmlns:c="{C}" xmlns:r="{R}" r:id="{rel_id}"/></a:graphicData></a:graphic>
</p:graphicFrame>'''


def slide_shapes(
    slide: dict,
    idx: int,
    image_rel_id: str | None = None,
    chart_rel_id: str | None = None,
) -> list[str]:
    kind = slide.get("type", "bullets")
    title = str(slide.get("title") or f"Slide {idx}")
    shapes = []
    next_id = 2
    if kind == "title":
        shapes.append(shape(next_id, "Title", [title], 914400, 1700000, 7315200, 900000, 3600))
        next_id += 1
        subtitle = slide.get("subtitle")
        if subtitle:
            shapes.append(shape(next_id, "Subtitle", [str(subtitle)], 1200000, 2700000, 6700000, 700000, 2200))
        return shapes
    if kind == "section":
        shapes.append(shape(next_id, "Section", [title], 914400, 2300000, 7315200, 900000, 3400))
        next_id += 1
        subtitle = slide.get("subtitle")
        if subtitle:
            shapes.append(shape(next_id, "Subtitle", [str(subtitle)], 1200000, 3300000, 6700000, 600000, 2000))
        return shapes
    shapes.append(shape(next_id, "Title", [title], 457200, 280000, 8229600, 650000, 3000))
    next_id += 1
    if kind == "two_column":
        left = [str(v) for v in slide.get("left", [])]
        right = [str(v) for v in slide.get("right", [])]
        shapes.append(shape(next_id, "Left", left, 650000, 1200000, 3800000, 4200000, 2000, True))
        next_id += 1
        shapes.append(shape(next_id, "Right", right, 4850000, 1200000, 3800000, 4200000, 2000, True))
        return shapes
    if kind == "native_chart" and chart_rel_id:
        subtitle = slide.get("subtitle") or slide.get("body")
        if subtitle:
            shapes.append(shape(next_id, "Chart Note", [str(subtitle)], 650000, 900000, 7900000, 360000, 1500))
            next_id += 1
        shapes.append(chart_frame(next_id, chart_rel_id, 850000, 1350000, 7500000, 3200000))
        return shapes
    if image_rel_id:
        shapes.append(picture(next_id, "Image", image_rel_id, 900000, 1250000, 7300000, 3000000))
        next_id += 1
        caption = slide.get("caption") or slide.get("body")
        if caption:
            shapes.append(shape(next_id, "Caption", [str(caption)], 900000, 4350000, 7300000, 420000, 1500))
        return shapes
    if kind == "metrics":
        metrics = slide.get("metrics") or []
        box_w = 2450000
        gap = 220000
        for m_idx, metric in enumerate(metrics[:3]):
            x0 = 650000 + m_idx * (box_w + gap)
            shapes.append(rect(next_id, f"Metric {m_idx + 1} Fill", x0, 1500000, box_w, 1550000, "EFF6FF", "BFDBFE"))
            next_id += 1
            label = metric.get("label", f"Metric {m_idx + 1}") if isinstance(metric, dict) else ""
            value = metric.get("value", metric) if isinstance(metric, dict) else metric
            delta = metric.get("delta", "") if isinstance(metric, dict) else ""
            shapes.append(shape(next_id, f"Metric {m_idx + 1} Label", [str(label)], x0 + 180000, 1660000, box_w - 360000, 300000, 1500))
            next_id += 1
            shapes.append(shape(next_id, f"Metric {m_idx + 1} Value", [str(value)], x0 + 180000, 2000000, box_w - 360000, 520000, 3100))
            next_id += 1
            if delta:
                shapes.append(shape(next_id, f"Metric {m_idx + 1} Delta", [str(delta)], x0 + 180000, 2560000, box_w - 360000, 300000, 1500))
                next_id += 1
        return shapes
    if kind == "table":
        headers = [str(v) for v in slide.get("headers", [])]
        rows = [[str(v) for v in row] for row in slide.get("rows", [])]
        col_count = max(len(headers), *(len(row) for row in rows), 1)
        row_count = min(1 + len(rows), 7)
        x0, y0, width, height = 650000, 1250000, 7900000, 3300000
        cell_w = int(width / col_count)
        cell_h = int(height / max(row_count, 1))
        table_rows = [headers] + rows
        for r_idx, row in enumerate(table_rows[:row_count]):
            for c_idx in range(col_count):
                cell_x = x0 + c_idx * cell_w
                cell_y = y0 + r_idx * cell_h
                fill = "EAF2F8" if r_idx == 0 else "FFFFFF"
                shapes.append(rect(next_id, f"Cell {r_idx + 1}-{c_idx + 1}", cell_x, cell_y, cell_w, cell_h, fill, "CBD5E1"))
                next_id += 1
                value = row[c_idx] if c_idx < len(row) else ""
                shapes.append(shape(next_id, f"Cell Text {r_idx + 1}-{c_idx + 1}", [value], cell_x + 80000, cell_y + 70000, cell_w - 160000, cell_h - 120000, 1200 if r_idx else 1300))
                next_id += 1
        return shapes
    if kind == "timeline":
        items = slide.get("items") or slide.get("milestones") or []
        y = 2750000
        shapes.append(connector(next_id, "Timeline Axis", 900000, y, 8200000, y, "94A3B8"))
        next_id += 1
        count = max(len(items), 1)
        for item_idx, item in enumerate(items[:6]):
            x0 = 900000 + int(item_idx * (7300000 / max(count - 1, 1)))
            shapes.append(rect(next_id, f"Milestone {item_idx + 1}", x0 - 90000, y - 90000, 180000, 180000, "2563EB"))
            next_id += 1
            label = item.get("label", item) if isinstance(item, dict) else item
            date = item.get("date", "") if isinstance(item, dict) else ""
            shapes.append(shape(next_id, f"Milestone {item_idx + 1} Date", [str(date)], x0 - 420000, y - 620000, 840000, 250000, 1200))
            next_id += 1
            shapes.append(shape(next_id, f"Milestone {item_idx + 1} Label", [str(label)], x0 - 520000, y + 220000, 1040000, 520000, 1250))
            next_id += 1
        return shapes
    if kind == "chart":
        data = slide.get("data") or []
        values = []
        for point in data:
            try:
                values.append(float(point.get("value", 0) if isinstance(point, dict) else point[1]))
            except Exception:
                values.append(0.0)
        max_value = max(values, default=1.0) or 1.0
        x0, y0, max_w, bar_h = 1800000, 1450000, 6200000, 360000
        for point_idx, point in enumerate(data[:7]):
            label = point.get("label", f"Item {point_idx + 1}") if isinstance(point, dict) else str(point[0])
            value = values[point_idx]
            y_bar = y0 + point_idx * 500000
            shapes.append(shape(next_id, f"Bar Label {point_idx + 1}", [str(label)], 650000, y_bar, 1050000, 300000, 1200))
            next_id += 1
            width = max(50000, int(max_w * value / max_value))
            shapes.append(rect(next_id, f"Bar {point_idx + 1}", x0, y_bar, width, bar_h, "2563EB"))
            next_id += 1
            shapes.append(shape(next_id, f"Bar Value {point_idx + 1}", [f"{value:g}"], x0 + width + 100000, y_bar, 800000, 300000, 1200))
            next_id += 1
        return shapes
    body = slide.get("body")
    if body:
        shapes.append(shape(next_id, "Body", [str(body)], 700000, 1100000, 7800000, 700000, 1900))
        next_id += 1
    bullets = [str(v) for v in slide.get("bullets", [])]
    shapes.append(shape(next_id, "Bullets", bullets, 850000, 1900000, 7600000, 3500000, 2100, True))
    return shapes


def slide_xml(
    slide: dict,
    idx: int,
    image_rel_id: str | None = None,
    chart_rel_id: str | None = None,
) -> str:
    shapes = "".join(slide_shapes(slide, idx, image_rel_id, chart_rel_id))
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="{P}" xmlns:a="{A}" xmlns:r="{R}">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
    {shapes}
  </p:spTree></p:cSld>
  <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sld>'''


def presentation_xml(count: int) -> str:
    ids = "".join(f'<p:sldId id="{255 + i}" r:id="rId{i}"/>' for i in range(1, count + 1))
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="{P}" xmlns:a="{A}" xmlns:r="{R}">
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId{count + 1}"/></p:sldMasterIdLst>
  <p:sldIdLst>{ids}</p:sldIdLst>
  <p:sldSz cx="9144000" cy="5143500" type="screen16x9"/>
  <p:notesSz cx="6858000" cy="9144000"/>
</p:presentation>'''


def presentation_rels(count: int) -> str:
    rels = []
    for idx in range(1, count + 1):
        rels.append(
            f'<Relationship Id="rId{idx}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{idx}.xml"/>'
        )
    rels.append(
        f'<Relationship Id="rId{count + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>'
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + "".join(rels)
        + "</Relationships>"
    )


def media_ext(path: Path) -> str:
    ext = path.suffix.lower().lstrip(".")
    if ext == "jpg":
        return "jpeg"
    return ext or "png"


def content_types(count: int, chart_count: int = 0) -> str:
    overrides = [
        '<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>',
        '<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>',
        '<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>',
        '<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>',
        '<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>',
        '<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>',
    ]
    for idx in range(1, count + 1):
        overrides.append(
            f'<Override PartName="/ppt/slides/slide{idx}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>'
        )
    for idx in range(1, chart_count + 1):
        overrides.append(
            f'<Override PartName="/ppt/charts/chart{idx}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>'
        )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Default Extension="png" ContentType="image/png"/>'
        '<Default Extension="jpg" ContentType="image/jpeg"/>'
        '<Default Extension="jpeg" ContentType="image/jpeg"/>'
        '<Default Extension="gif" ContentType="image/gif"/>'
        + "".join(overrides)
        + "</Types>"
    )


def core_xml(title: str) -> str:
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>{x(title)}</dc:title><dc:creator>Hope Agent</dc:creator>
  <dcterms:created xsi:type="dcterms:W3CDTF">{now}</dcterms:created>
  <dcterms:modified xsi:type="dcterms:W3CDTF">{now}</dcterms:modified>
</cp:coreProperties>'''


SLIDE_MASTER = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldMaster xmlns:p="{P}" xmlns:a="{A}" xmlns:r="{R}"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>'''
SLIDE_MASTER_RELS = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>'''
SLIDE_LAYOUT = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldLayout xmlns:p="{P}" xmlns:a="{A}" xmlns:r="{R}" type="blank"><p:cSld name="Blank"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>'''
SLIDE_LAYOUT_RELS = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>'''
THEME = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="{A}" name="Hope"><a:themeElements><a:clrScheme name="Hope"><a:dk1><a:srgbClr val="1F2937"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="374151"/></a:dk2><a:lt2><a:srgbClr val="F8FAFC"/></a:lt2><a:accent1><a:srgbClr val="2563EB"/></a:accent1><a:accent2><a:srgbClr val="059669"/></a:accent2><a:accent3><a:srgbClr val="D97706"/></a:accent3><a:accent4><a:srgbClr val="7C3AED"/></a:accent4><a:accent5><a:srgbClr val="DB2777"/></a:accent5><a:accent6><a:srgbClr val="0891B2"/></a:accent6><a:hlink><a:srgbClr val="2563EB"/></a:hlink><a:folHlink><a:srgbClr val="7C3AED"/></a:folHlink></a:clrScheme><a:fontScheme name="Hope"><a:majorFont><a:latin typeface="Aptos Display"/></a:majorFont><a:minorFont><a:latin typeface="Aptos"/></a:minorFont></a:fontScheme><a:fmtScheme name="Hope"><a:fillStyleLst/><a:lnStyleLst/><a:effectStyleLst/><a:bgFillStyleLst/></a:fmtScheme></a:themeElements></a:theme>'''


def slide_image_path(slide: dict, base_dir: Path) -> Path | None:
    raw = slide.get("image")
    if not raw:
        return None
    path = Path(str(raw))
    if not path.is_absolute():
        path = base_dir / path
    return path


def slide_rels(image_target: str | None = None, chart_target: str | None = None) -> str:
    image_rel = (
        f'<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{image_target}"/>'
        if image_target
        else ""
    )
    chart_rel_id = "rId3" if image_target else "rId2"
    chart_rel = (
        f'<Relationship Id="{chart_rel_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="{chart_target}"/>'
        if chart_target
        else ""
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>'
        + image_rel
        + chart_rel
        + "</Relationships>"
    )


def chart_points(slide: dict) -> list[tuple[str, float]]:
    points = []
    for idx, point in enumerate(slide.get("data") or [], 1):
        if isinstance(point, dict):
            label = str(point.get("label", f"Item {idx}"))
            raw_value = point.get("value", 0)
        else:
            label = str(point[0] if point else f"Item {idx}")
            raw_value = point[1] if len(point) > 1 else 0
        try:
            value = float(raw_value)
        except Exception:
            value = 0.0
        points.append((label, value))
    return points or [("Value", 0.0)]


def chart_cache(points: list[tuple[str, float]]) -> tuple[str, str]:
    str_pts = "".join(
        f'<c:pt idx="{idx}"><c:v>{x(label)}</c:v></c:pt>' for idx, (label, _value) in enumerate(points)
    )
    num_pts = "".join(
        f'<c:pt idx="{idx}"><c:v>{value:g}</c:v></c:pt>' for idx, (_label, value) in enumerate(points)
    )
    cat = f'<c:cat><c:strLit><c:ptCount val="{len(points)}"/>{str_pts}</c:strLit></c:cat>'
    val = f'<c:val><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="{len(points)}"/>{num_pts}</c:numLit></c:val>'
    return cat, val


def native_chart_xml(slide: dict) -> str:
    title = str(slide.get("title") or "Chart")
    kind = str(slide.get("chart_type") or slide.get("kind") or "bar").lower()
    points = chart_points(slide)
    cat, val = chart_cache(points)
    ser = f'<c:ser><c:idx val="0"/><c:order val="0"/>{cat}{val}</c:ser>'
    if kind in {"line", "line_chart"}:
        plot = f'<c:lineChart><c:grouping val="standard"/>{ser}<c:axId val="48650112"/><c:axId val="48672768"/></c:lineChart>{chart_axes_xml()}'
    elif kind in {"pie", "pie_chart"}:
        plot = f'<c:pieChart>{ser}<c:firstSliceAng val="0"/></c:pieChart>'
    else:
        direction = "bar" if kind in {"horizontal_bar", "bar"} else "col"
        plot = f'<c:barChart><c:barDir val="{direction}"/><c:grouping val="clustered"/>{ser}<c:axId val="48650112"/><c:axId val="48672768"/></c:barChart>{chart_axes_xml()}'
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="{C}" xmlns:a="{A}" xmlns:r="{R}">
  <c:chart>
    <c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{x(title)}</a:t></a:r></a:p></c:rich></c:tx><c:layout/></c:title>
    <c:plotArea><c:layout/>{plot}</c:plotArea>
    <c:legend><c:legendPos val="r"/><c:layout/></c:legend>
    <c:plotVisOnly val="1"/>
  </c:chart>
</c:chartSpace>'''


def chart_axes_xml() -> str:
    return '''<c:catAx><c:axId val="48650112"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="b"/><c:tickLblPos val="nextTo"/><c:crossAx val="48672768"/><c:crosses val="autoZero"/></c:catAx>
      <c:valAx><c:axId val="48672768"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:axPos val="l"/><c:majorGridlines/><c:tickLblPos val="nextTo"/><c:crossAx val="48650112"/><c:crosses val="autoZero"/></c:valAx>'''


def write_pptx(spec: dict, out: Path) -> None:
    slides = spec.get("slides") or [{"type": "title", "title": spec.get("title") or out.stem}]
    out.parent.mkdir(parents=True, exist_ok=True)
    count = len(slides)
    chart_slides = [slide for slide in slides if slide.get("type") == "native_chart"]
    entries = {
        "[Content_Types].xml": content_types(count, len(chart_slides)),
        "_rels/.rels": '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>''',
        "docProps/app.xml": '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Hope Agent</Application></Properties>''',
        "docProps/core.xml": core_xml(str(spec.get("title") or out.stem)),
        "ppt/presentation.xml": presentation_xml(count),
        "ppt/_rels/presentation.xml.rels": presentation_rels(count),
        "ppt/slideMasters/slideMaster1.xml": SLIDE_MASTER,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels": SLIDE_MASTER_RELS,
        "ppt/slideLayouts/slideLayout1.xml": SLIDE_LAYOUT,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels": SLIDE_LAYOUT_RELS,
        "ppt/theme/theme1.xml": THEME,
    }
    media_idx = 1
    chart_idx = 1
    base_dir = out.parent
    for idx, slide in enumerate(slides, 1):
        image_path = slide_image_path(slide, base_dir)
        image_rel_id = None
        image_target = None
        if image_path:
            ext = media_ext(image_path)
            media_name = f"image{media_idx}.{ext if ext != 'jpeg' else 'jpg'}"
            entries[f"ppt/media/{media_name}"] = image_path.read_bytes()
            image_target = f"../media/{media_name}"
            image_rel_id = "rId2"
            media_idx += 1
        chart_rel_id = None
        chart_target = None
        if slide.get("type") == "native_chart":
            chart_name = f"chart{chart_idx}.xml"
            entries[f"ppt/charts/{chart_name}"] = native_chart_xml(slide)
            chart_target = f"../charts/{chart_name}"
            chart_rel_id = "rId3" if image_target else "rId2"
            chart_idx += 1
        entries[f"ppt/slides/slide{idx}.xml"] = slide_xml(slide, idx, image_rel_id, chart_rel_id)
        entries[f"ppt/slides/_rels/slide{idx}.xml.rels"] = slide_rels(image_target, chart_target)
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries.items():
            zf.writestr(name, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--spec", required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    write_pptx(json.loads(Path(ns.spec).read_text(encoding="utf-8")), Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
