#!/usr/bin/env python3
"""Build an editable DOCX from a JSON spec using only Python stdlib.

The builder intentionally writes deterministic OOXML instead of relying on
external packages. It supports real Word numbering, comments, tracked changes,
tables, callouts, and predictable style parts so generated files remain
editable in Word, LibreOffice, and Google Docs import flows.
"""

from __future__ import annotations

import argparse
import json
import mimetypes
import zipfile
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from xml.sax.saxutils import escape


NS_W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
NS_R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
NS_WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
NS_A = "http://schemas.openxmlformats.org/drawingml/2006/main"
NS_PIC = "http://schemas.openxmlformats.org/drawingml/2006/picture"


def x(text: object) -> str:
    return escape("" if text is None else str(text))


def now_w3c() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


@dataclass
class Comment:
    id: int
    text: str
    author: str = "Hope Agent"
    initials: str = "HA"
    date: str = field(default_factory=now_w3c)


@dataclass
class ImagePart:
    rel_id: str
    target: str
    source: Path
    alt: str
    name: str
    content_type: str
    width_emu: int
    height_emu: int


@dataclass
class BuildContext:
    next_comment_id: int = 0
    next_revision_id: int = 1
    next_image_id: int = 1
    base_dir: Path = field(default_factory=Path.cwd)
    comments: list[Comment] = field(default_factory=list)
    images: list[ImagePart] = field(default_factory=list)
    has_revisions: bool = False

    def add_comment(self, text: object, author: str = "Hope Agent", initials: str = "HA") -> int:
        comment_id = self.next_comment_id
        self.next_comment_id += 1
        self.comments.append(Comment(comment_id, str(text or ""), author, initials))
        return comment_id

    def revision_id(self) -> int:
        value = self.next_revision_id
        self.next_revision_id += 1
        self.has_revisions = True
        return value

    def add_image(
        self,
        raw_path: object,
        *,
        alt: object = "",
        width_inches: float = 5.5,
        height_inches: float = 3.2,
    ) -> ImagePart:
        path = Path(str(raw_path))
        if not path.is_absolute():
            path = self.base_dir / path
        if not path.exists():
            raise FileNotFoundError(path)
        image_id = self.next_image_id
        self.next_image_id += 1
        ext = path.suffix.lower().lstrip(".") or "png"
        if ext == "jpg":
            ext = "jpeg"
        rel_id = f"rIdImage{image_id}"
        target = f"media/image{image_id}.{ext if ext != 'jpeg' else 'jpg'}"
        content_type = mimetypes.types_map.get(f".{ext}", f"image/{ext}")
        image = ImagePart(
            rel_id=rel_id,
            target=target,
            source=path,
            alt=str(alt or ""),
            name=f"Picture {image_id}",
            content_type=content_type,
            width_emu=int(float(width_inches or 5.5) * 914400),
            height_emu=int(float(height_inches or 3.2) * 914400),
        )
        self.images.append(image)
        return image


def text_run(text: object) -> str:
    raw = "" if text is None else str(text)
    preserve = ' xml:space="preserve"' if raw != raw.strip() else ""
    return f"<w:r><w:t{preserve}>{x(raw)}</w:t></w:r>"


def revision_run(text: object, ctx: BuildContext, kind: str) -> str:
    rev_id = ctx.revision_id()
    date = now_w3c()
    raw = "" if text is None else str(text)
    preserve = ' xml:space="preserve"' if raw != raw.strip() else ""
    if kind == "delete":
        return (
            f'<w:del w:id="{rev_id}" w:author="Hope Agent" w:date="{date}">'
            f"<w:r><w:delText{preserve}>{x(raw)}</w:delText></w:r></w:del>"
        )
    return (
        f'<w:ins w:id="{rev_id}" w:author="Hope Agent" w:date="{date}">'
        + text_run(raw)
        + "</w:ins>"
    )


def paragraph(
    text: object,
    style: str | None = None,
    *,
    ctx: BuildContext | None = None,
    num_id: int | None = None,
    ilvl: int = 0,
    comment: object | None = None,
    inserted: bool = False,
    deleted: bool = False,
) -> str:
    ctx = ctx or BuildContext()
    ppr = []
    if style:
        ppr.append(f'<w:pStyle w:val="{x(style)}"/>')
    if num_id is not None:
        ppr.append(
            f'<w:numPr><w:ilvl w:val="{int(ilvl)}"/><w:numId w:val="{int(num_id)}"/></w:numPr>'
        )
    ppr_xml = f"<w:pPr>{''.join(ppr)}</w:pPr>" if ppr else ""
    if deleted:
        run_xml = revision_run(text, ctx, "delete")
    elif inserted:
        run_xml = revision_run(text, ctx, "insert")
    else:
        run_xml = text_run(text)
    if comment:
        comment_id = ctx.add_comment(
            comment.get("text", comment) if isinstance(comment, dict) else comment,
            str(comment.get("author", "Hope Agent")) if isinstance(comment, dict) else "Hope Agent",
            str(comment.get("initials", "HA")) if isinstance(comment, dict) else "HA",
        )
        run_xml = (
            f'<w:commentRangeStart w:id="{comment_id}"/>'
            + run_xml
            + f'<w:commentRangeEnd w:id="{comment_id}"/>'
            + f'<w:r><w:commentReference w:id="{comment_id}"/></w:r>'
        )
    return f"<w:p>{ppr_xml}{run_xml}</w:p>"


def image_paragraph(block: dict, *, ctx: BuildContext) -> str:
    image = ctx.add_image(
        block.get("path") or block.get("image"),
        alt=block.get("alt", block.get("caption", "")),
        width_inches=float(block.get("width_inches", 5.5) or 5.5),
        height_inches=float(block.get("height_inches", 3.2) or 3.2),
    )
    doc_pr_id = ctx.next_image_id + 1000
    caption = block.get("caption")
    drawing = f'''<w:p><w:r><w:drawing>
  <wp:inline xmlns:wp="{NS_WP}" xmlns:a="{NS_A}" xmlns:pic="{NS_PIC}" distT="0" distB="0" distL="0" distR="0">
    <wp:extent cx="{image.width_emu}" cy="{image.height_emu}"/>
    <wp:docPr id="{doc_pr_id}" name="{x(image.name)}" descr="{x(image.alt)}"/>
    <wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr>
    <a:graphic><a:graphicData uri="{NS_PIC}">
      <pic:pic>
        <pic:nvPicPr><pic:cNvPr id="0" name="{x(image.source.name)}" descr="{x(image.alt)}"/><pic:cNvPicPr/></pic:nvPicPr>
        <pic:blipFill><a:blip r:embed="{image.rel_id}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>
        <pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{image.width_emu}" cy="{image.height_emu}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>
      </pic:pic>
    </a:graphicData></a:graphic>
  </wp:inline>
</w:drawing></w:r></w:p>'''
    if caption:
        drawing += paragraph(str(caption), "Caption", ctx=ctx)
    return drawing


def heading(text: object, level: int = 1, *, ctx: BuildContext | None = None) -> str:
    level = max(1, min(int(level or 1), 3))
    return paragraph(text, f"Heading{level}", ctx=ctx)


def list_items(items: list[object], num_id: int, *, ctx: BuildContext) -> str:
    xml = []
    for item in items:
        if isinstance(item, dict):
            xml.append(
                paragraph(
                    item.get("text", ""),
                    ctx=ctx,
                    num_id=num_id,
                    ilvl=int(item.get("level", 0) or 0),
                    comment=item.get("comment"),
                )
            )
        else:
            xml.append(paragraph(item, ctx=ctx, num_id=num_id))
    return "".join(xml)


def table(headers: list[object], rows: list[list[object]], *, widths: list[int] | None = None) -> str:
    col_count = max(len(headers), *(len(row) for row in rows), 1)
    if widths:
        col_widths = [int(w) for w in widths[:col_count]]
        if len(col_widths) < col_count:
            col_widths.extend([2400] * (col_count - len(col_widths)))
    else:
        width = max(1200, int(9360 / col_count))
        col_widths = [width] * col_count

    def cell(value: object, idx: int, header: bool = False) -> str:
        shade = '<w:shd w:fill="EAF2F8"/>' if header else ""
        bold = '<w:rPr><w:b/></w:rPr>' if header else ""
        text = "" if value is None else str(value)
        return (
            f'<w:tc><w:tcPr><w:tcW w:w="{col_widths[idx]}" w:type="dxa"/>{shade}'
            '<w:tcMar><w:top w:w="120" w:type="dxa"/><w:left w:w="120" w:type="dxa"/>'
            '<w:bottom w:w="120" w:type="dxa"/><w:right w:w="120" w:type="dxa"/></w:tcMar>'
            f"</w:tcPr><w:p><w:r>{bold}<w:t>{x(text)}</w:t></w:r></w:p></w:tc>"
        )

    xml = [
        '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/>'
        '<w:tblW w:w="9360" w:type="dxa"/><w:tblLook w:firstRow="1" w:noHBand="0"/>'
        "</w:tblPr><w:tblGrid>"
    ]
    xml.extend(f'<w:gridCol w:w="{width}"/>' for width in col_widths)
    xml.append("</w:tblGrid>")
    if headers:
        xml.append("<w:tr>")
        padded = list(headers) + [""] * (col_count - len(headers))
        xml.extend(cell(value, idx, True) for idx, value in enumerate(padded[:col_count]))
        xml.append("</w:tr>")
    for row in rows:
        xml.append("<w:tr>")
        padded = list(row) + [""] * (col_count - len(row))
        xml.extend(cell(value, idx) for idx, value in enumerate(padded[:col_count]))
        xml.append("</w:tr>")
    xml.append("</w:tbl>")
    return "".join(xml)


def callout(block: dict, *, ctx: BuildContext) -> str:
    label = block.get("label", "Note")
    text = block.get("text", "")
    return table([], [[f"{label}: {text}"]], widths=[9360])


def block_xml(block: dict, ctx: BuildContext | None = None) -> str:
    ctx = ctx or BuildContext()
    kind = block.get("type", "paragraph")
    if kind == "heading":
        return heading(block.get("text", ""), block.get("level", 1), ctx=ctx)
    if kind == "bullet_list":
        return list_items(block.get("items", []), 1, ctx=ctx)
    if kind == "numbered_list":
        return list_items(block.get("items", []), 2, ctx=ctx)
    if kind == "table":
        return table(
            block.get("headers", []),
            block.get("rows", []),
            widths=block.get("column_widths"),
        )
    if kind == "callout":
        return callout(block, ctx=ctx)
    if kind == "image":
        return image_paragraph(block, ctx=ctx)
    if kind == "page_break":
        return '<w:p><w:r><w:br w:type="page"/></w:r></w:p>'
    if kind == "comment":
        return paragraph(
            block.get("target", block.get("text", "")),
            ctx=ctx,
            comment={
                "text": block.get("comment", block.get("note", "")),
                "author": block.get("author", "Hope Agent"),
                "initials": block.get("initials", "HA"),
            },
        )
    if kind == "revision":
        old_text = block.get("delete", block.get("old_text", ""))
        new_text = block.get("insert", block.get("new_text", ""))
        return paragraph(old_text, ctx=ctx, deleted=True) + paragraph(new_text, ctx=ctx, inserted=True)
    return paragraph(
        block.get("text", ""),
        block.get("style"),
        ctx=ctx,
        comment=block.get("comment"),
        inserted=bool(block.get("inserted")),
        deleted=bool(block.get("deleted")),
    )


def document_xml(spec: dict, ctx: BuildContext) -> str:
    body: list[str] = []
    title = spec.get("title")
    subtitle = spec.get("subtitle")
    if title:
        body.append(paragraph(title, "DocTitle", ctx=ctx))
    if subtitle:
        body.append(paragraph(subtitle, "Subtitle", ctx=ctx))
    blocks = spec.get("blocks") or []
    if not blocks and spec.get("paragraphs"):
        blocks = [{"type": "paragraph", "text": p} for p in spec["paragraphs"]]
    body.extend(block_xml(block, ctx) for block in blocks)
    body.append(
        '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/>'
        '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" '
        'w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>'
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body>'
        + "".join(body)
        + "</w:body></w:document>"
    )


def styles_xml() -> str:
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="{NS_W}">
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:pPr><w:spacing w:after="120" w:line="276" w:lineRule="auto"/></w:pPr><w:rPr><w:rFonts w:ascii="Aptos" w:hAnsi="Aptos"/><w:sz w:val="22"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="DocTitle"><w:name w:val="Document Title"/><w:pPr><w:spacing w:before="0" w:after="160"/></w:pPr><w:rPr><w:rFonts w:ascii="Aptos Display" w:hAnsi="Aptos Display"/><w:b/><w:sz w:val="38"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Subtitle"><w:name w:val="Subtitle"/><w:pPr><w:spacing w:after="220"/></w:pPr><w:rPr><w:i/><w:color w:val="5B677A"/><w:sz w:val="24"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="280" w:after="120"/></w:pPr><w:rPr><w:b/><w:color w:val="1F4E79"/><w:sz w:val="30"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="220" w:after="100"/></w:pPr><w:rPr><w:b/><w:sz w:val="26"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="180" w:after="80"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Callout"><w:name w:val="Callout"/><w:rPr><w:b/><w:color w:val="1F4E79"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="Caption"/><w:pPr><w:spacing w:before="80" w:after="160"/></w:pPr><w:rPr><w:i/><w:color w:val="64748B"/><w:sz w:val="18"/></w:rPr></w:style>
  <w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="C9D8EA"/><w:left w:val="single" w:sz="4" w:space="0" w:color="C9D8EA"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="C9D8EA"/><w:right w:val="single" w:sz="4" w:space="0" w:color="C9D8EA"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="C9D8EA"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="C9D8EA"/></w:tblBorders></w:tblPr></w:style>
</w:styles>'''


def numbering_xml() -> str:
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="{NS_W}">
  <w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
  <w:abstractNum w:abstractNumId="2"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
  <w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num>
  <w:num w:numId="2"><w:abstractNumId w:val="2"/></w:num>
</w:numbering>'''


def comments_xml(comments: list[Comment]) -> str:
    body = []
    for comment in comments:
        body.append(
            f'<w:comment w:id="{comment.id}" w:author="{x(comment.author)}" '
            f'w:initials="{x(comment.initials)}" w:date="{comment.date}">'
            f"<w:p>{text_run(comment.text)}</w:p></w:comment>"
        )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<w:comments xmlns:w="{NS_W}">'
        + "".join(body)
        + "</w:comments>"
    )


def settings_xml(track_revisions: bool) -> str:
    track = "<w:trackRevisions/>" if track_revisions else ""
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<w:settings xmlns:w="{NS_W}"><w:zoom w:percent="100"/>{track}</w:settings>'
    )


def core_xml(title: str) -> str:
    now = now_w3c()
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>{x(title)}</dc:title><dc:creator>Hope Agent</dc:creator>
  <cp:lastModifiedBy>Hope Agent</cp:lastModifiedBy>
  <dcterms:created xsi:type="dcterms:W3CDTF">{now}</dcterms:created>
  <dcterms:modified xsi:type="dcterms:W3CDTF">{now}</dcterms:modified>
</cp:coreProperties>'''


def image_defaults_xml(images: list[ImagePart]) -> str:
    defaults = []
    seen = set()
    for image in images:
        ext = Path(image.target).suffix.lower().lstrip(".")
        if ext not in seen:
            seen.add(ext)
            defaults.append(f'<Default Extension="{x(ext)}" ContentType="{x(image.content_type)}"/>')
    return "".join(defaults)


def content_types_xml(include_comments: bool, images: list[ImagePart] | None = None) -> str:
    images = images or []
    comments = (
        '<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/>'
        if include_comments
        else ""
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        + image_defaults_xml(images)
        + '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
        '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>'
        '<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>'
        '<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>'
        + comments
        + '<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>'
        + '<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>'
        "</Types>"
    )


def document_rels_xml(include_comments: bool, images: list[ImagePart] | None = None) -> str:
    images = images or []
    comments = (
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>'
        if include_comments
        else ""
    )
    image_rels = "".join(
        f'<Relationship Id="{x(image.rel_id)}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{x(image.target)}"/>'
        for image in images
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>'
        + comments
        + image_rels
        + "</Relationships>"
    )


def write_docx(spec: dict, out: Path, base_dir: Path | None = None) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    title = str(spec.get("title") or out.stem)
    ctx = BuildContext(base_dir=base_dir or Path.cwd())
    doc_xml = document_xml(spec, ctx)
    include_comments = bool(ctx.comments)
    entries = {
        "[Content_Types].xml": content_types_xml(include_comments, ctx.images),
        "_rels/.rels": '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>''',
        "docProps/app.xml": '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Hope Agent</Application></Properties>''',
        "docProps/core.xml": core_xml(title),
        "word/_rels/document.xml.rels": document_rels_xml(include_comments, ctx.images),
        "word/document.xml": doc_xml,
        "word/numbering.xml": numbering_xml(),
        "word/settings.xml": settings_xml(bool(spec.get("track_revisions")) or ctx.has_revisions),
        "word/styles.xml": styles_xml(),
    }
    if include_comments:
        entries["word/comments.xml"] = comments_xml(ctx.comments)
    for image in ctx.images:
        entries[f"word/{image.target}"] = image.source.read_bytes()
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries.items():
            zf.writestr(name, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--spec", required=True, help="JSON spec path")
    parser.add_argument("--out", required=True, help="Output .docx path")
    ns = parser.parse_args()
    spec_path = Path(ns.spec)
    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    write_docx(spec, Path(ns.out), spec_path.parent)
    print(ns.out)


if __name__ == "__main__":
    main()
