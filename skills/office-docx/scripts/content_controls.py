#!/usr/bin/env python3
"""Wrap placeholder text in simple DOCX rich-text content controls."""

from __future__ import annotations

import argparse
import json
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape


def insert_before_final_section(xml: str, payload: str) -> str:
    sect_idx = xml.rfind("<w:sectPr")
    if sect_idx >= 0:
        return xml[:sect_idx] + payload + xml[sect_idx:]
    return xml.replace("</w:body>", payload + "</w:body>")


def control_xml(tag: str, alias: str, text: str) -> str:
    return (
        f'<w:sdt><w:sdtPr><w:alias w:val="{escape(alias)}"/><w:tag w:val="{escape(tag)}"/></w:sdtPr>'
        f'<w:sdtContent><w:p><w:r><w:t>{escape(text)}</w:t></w:r></w:p></w:sdtContent></w:sdt>'
    )


def add_controls(src: Path, spec: dict, out: Path) -> None:
    controls = spec.get("controls") or []
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if item.filename == "word/document.xml":
                xml = data.decode("utf-8")
                insert = "".join(control_xml(c.get("tag", c.get("alias", "field")), c.get("alias", c.get("tag", "Field")), c.get("text", "")) for c in controls)
                xml = insert_before_final_section(xml, insert)
                data = xml.encode("utf-8")
            zout.writestr(item, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--spec", required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    add_controls(Path(ns.input), json.loads(Path(ns.spec).read_text(encoding="utf-8")), Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
