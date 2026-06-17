#!/usr/bin/env python3
"""Add a simple text watermark to DOCX headers."""

from __future__ import annotations

import argparse
import re
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape


HEADER_XML = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:p><w:r><w:t>{text}</w:t></w:r></w:p>
</w:hdr>'''


def next_header_name(names: set[str]) -> str:
    nums = []
    for name in names:
        match = re.fullmatch(r"word/header([0-9]+)\.xml", name)
        if match:
            nums.append(int(match.group(1)))
    return f"header{max(nums, default=0) + 1}.xml"


def next_rel_id(rels: str) -> str:
    ids = [int(value) for value in re.findall(r'Id="rId([0-9]+)"', rels)]
    return f"rId{max(ids, default=0) + 1}"


def add_header_reference(document_xml: str, rel_id: str) -> str:
    reference = f'<w:headerReference w:type="default" r:id="{rel_id}"/>'
    return re.sub(r"(<w:sectPr\b[^>]*>)", r"\1" + reference, document_xml, count=1)


def add_watermark(src: Path, out: Path, text: str) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin:
        names = set(zin.namelist())
        header_name = next_header_name(names)
        rels_name = "word/_rels/document.xml.rels"
        rels = (
            zin.read(rels_name).decode("utf-8")
            if rels_name in names
            else '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>'
        )
        rel_id = next_rel_id(rels)
        rels = rels.replace(
            "</Relationships>",
            f'<Relationship Id="{rel_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="{header_name}"/></Relationships>',
        )

        with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
            written = set()
            for item in zin.infolist():
                written.add(item.filename)
                data = zin.read(item.filename)
                if item.filename == "word/document.xml":
                    xml = data.decode("utf-8")
                    xml = add_header_reference(xml, rel_id)
                    data = xml.encode("utf-8")
                elif item.filename == rels_name:
                    data = rels.encode("utf-8")
                elif item.filename == "[Content_Types].xml":
                    xml = data.decode("utf-8")
                    part_name = f"/word/{header_name}"
                    if part_name not in xml:
                        xml = xml.replace("</Types>", f'<Override PartName="{part_name}" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>')
                    data = xml.encode("utf-8")
                zout.writestr(item, data)
            if rels_name not in written:
                zout.writestr(rels_name, rels)
            zout.writestr(f"word/{header_name}", HEADER_XML.format(text=escape(text)))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--text", required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    add_watermark(Path(ns.input), Path(ns.out), ns.text)
    print(ns.out)


if __name__ == "__main__":
    main()
