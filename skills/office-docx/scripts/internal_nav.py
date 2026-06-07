#!/usr/bin/env python3
"""Append bookmarks and hyperlinks to a DOCX."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape


def insert_before_final_section(xml: str, payload: str) -> str:
    sect_idx = xml.rfind("<w:sectPr")
    if sect_idx >= 0:
        return xml[:sect_idx] + payload + xml[sect_idx:]
    return xml.replace("</w:body>", payload + "</w:body>")


def next_rel_id(rels: str) -> str:
    ids = [int(value) for value in re.findall(r'Id="rId([0-9]+)"', rels)]
    return f"rId{max(ids, default=0) + 1}"


def link_paragraph(text: str, *, rel_id: str | None = None, anchor: str | None = None) -> str:
    attr = f'r:id="{rel_id}"' if rel_id else f'w:anchor="{escape(anchor or "")}"'
    return (
        f'<w:p><w:hyperlink {attr}><w:r><w:rPr><w:u w:val="single"/><w:color w:val="0563C1"/></w:rPr>'
        f"<w:t>{escape(text)}</w:t></w:r></w:hyperlink></w:p>"
    )


def bookmark_paragraph(name: str, text: str, idx: int) -> str:
    return (
        f'<w:p><w:bookmarkStart w:id="{idx}" w:name="{escape(name)}"/>'
        f"<w:r><w:t>{escape(text)}</w:t></w:r><w:bookmarkEnd w:id=\"{idx}\"/></w:p>"
    )


def patch(src: Path, spec: dict, out: Path) -> dict:
    out.parent.mkdir(parents=True, exist_ok=True)
    external_links = [link for link in spec.get("links", []) if link.get("url")]
    with zipfile.ZipFile(src) as zin:
        rels = zin.read("word/_rels/document.xml.rels").decode("utf-8")
        link_rels = []
        for link in external_links:
            rid = next_rel_id(rels)
            rels = rels.replace(
                "</Relationships>",
                f'<Relationship Id="{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{escape(str(link["url"]))}" TargetMode="External"/></Relationships>',
            )
            link_rels.append((link, rid))
        with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
            for item in zin.infolist():
                data = zin.read(item.filename)
                if item.filename == "word/document.xml":
                    xml = data.decode("utf-8")
                    payload = []
                    for idx, bm in enumerate(spec.get("bookmarks", []), 1):
                        payload.append(bookmark_paragraph(str(bm.get("name", f"bookmark{idx}")), str(bm.get("text", bm.get("name", ""))), idx))
                    for link, rid in link_rels:
                        payload.append(link_paragraph(str(link.get("text", link.get("url"))), rel_id=rid))
                    for link in [link for link in spec.get("links", []) if link.get("anchor")]:
                        payload.append(link_paragraph(str(link.get("text", link.get("anchor"))), anchor=str(link["anchor"])))
                    xml = insert_before_final_section(xml, "".join(payload))
                    data = xml.encode("utf-8")
                elif item.filename == "word/_rels/document.xml.rels":
                    data = rels.encode("utf-8")
                zout.writestr(item, data)
    return {"bookmarks": len(spec.get("bookmarks", [])), "links": len(spec.get("links", [])), "out": str(out)}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--spec", required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    spec = json.loads(Path(ns.spec).read_text(encoding="utf-8"))
    print(json.dumps(patch(Path(ns.input), spec, Path(ns.out)), indent=2))


if __name__ == "__main__":
    main()
