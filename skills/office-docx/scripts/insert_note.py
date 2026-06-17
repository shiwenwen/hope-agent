#!/usr/bin/env python3
"""Append a simple footnote or endnote reference to a DOCX."""

from __future__ import annotations

import argparse
import re
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape


def ensure_override(content_types: str, part: str, content_type: str) -> str:
    if part in content_types:
        return content_types
    return content_types.replace("</Types>", f'<Override PartName="{part}" ContentType="{content_type}"/></Types>')


def ensure_rel(rels: str, rel_id: str, rel_type: str, target: str) -> str:
    if rel_type in rels and f'Target="{target}"' in rels:
        return rels
    return rels.replace("</Relationships>", f'<Relationship Id="{rel_id}" Type="{rel_type}" Target="{target}"/></Relationships>')


def note_part(kind: str, note_id: int, text: str) -> str:
    root = "footnotes" if kind == "footnote" else "endnotes"
    item = "footnote" if kind == "footnote" else "endnote"
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<w:{root} xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
        f'<w:{item} w:id="{note_id}"><w:p><w:r><w:t>{escape(text)}</w:t></w:r></w:p></w:{item}>'
        f"</w:{root}>"
    )


def insert_note(src: Path, out: Path, kind: str, text: str) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    part = "word/footnotes.xml" if kind == "footnote" else "word/endnotes.xml"
    item = "footnote" if kind == "footnote" else "endnote"
    root = "footnotes" if kind == "footnote" else "endnotes"
    ref_tag = "footnoteReference" if kind == "footnote" else "endnoteReference"
    rel_type = f"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{root}"
    content_type = f"application/vnd.openxmlformats-officedocument.wordprocessingml.{root}+xml"
    with zipfile.ZipFile(src) as zin:
        existing = zin.read(part).decode("utf-8") if part in zin.namelist() else None
        ids = [int(v) for v in re.findall(r'w:id="([0-9]+)"', existing or "")]
        note_id = max(ids, default=1) + 1
        note_xml = note_part(kind, note_id, text) if existing is None else existing.replace(
            f"</w:{root}>",
            f'<w:{item} w:id="{note_id}"><w:p><w:r><w:t>{escape(text)}</w:t></w:r></w:p></w:{item}></w:{root}>',
        )
        with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
            written = set()
            for info in zin.infolist():
                data = zin.read(info.filename)
                written.add(info.filename)
                if info.filename == "word/document.xml":
                    xml = data.decode("utf-8")
                    marker = f'<w:r><w:{ref_tag} w:id="{note_id}"/></w:r>'
                    xml = xml.replace("</w:p>", marker + "</w:p>", 1)
                    data = xml.encode("utf-8")
                elif info.filename == "[Content_Types].xml":
                    data = ensure_override(data.decode("utf-8"), "/" + part, content_type).encode("utf-8")
                elif info.filename == "word/_rels/document.xml.rels":
                    data = ensure_rel(data.decode("utf-8"), f"rId{root.title()}", rel_type, f"{root}.xml").encode("utf-8")
                elif info.filename == part:
                    data = note_xml.encode("utf-8")
                zout.writestr(info, data)
            if part not in written:
                zout.writestr(part, note_xml)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--kind", choices=["footnote", "endnote"], default="footnote")
    parser.add_argument("--text", required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    insert_note(Path(ns.input), Path(ns.out), ns.kind, ns.text)
    print(ns.out)


if __name__ == "__main__":
    main()
