#!/usr/bin/env python3
"""Merge DOCX files by appending body content from later files."""

from __future__ import annotations

import argparse
import re
import zipfile
from pathlib import Path


def body_payload(path: Path) -> str:
    with zipfile.ZipFile(path) as zf:
        xml = zf.read("word/document.xml").decode("utf-8")
    body = re.search(r"<w:body>(.*)</w:body>", xml, flags=re.DOTALL)
    if not body:
        return ""
    payload = body.group(1)
    payload = re.sub(r"<w:sectPr\b.*?</w:sectPr>", "", payload, flags=re.DOTALL)
    return payload


def merge(inputs: list[Path], out: Path) -> None:
    if not inputs:
        raise SystemExit("At least one input is required")
    out.parent.mkdir(parents=True, exist_ok=True)
    inserts = [body_payload(path) for path in inputs[1:]]
    with zipfile.ZipFile(inputs[0]) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if item.filename == "word/document.xml":
                xml = data.decode("utf-8")
                sect = xml.rfind("<w:sectPr")
                if sect >= 0:
                    xml = xml[:sect] + "".join(inserts) + xml[sect:]
                else:
                    xml = xml.replace("</w:body>", "".join(inserts) + "</w:body>")
                data = xml.encode("utf-8")
            zout.writestr(item, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", action="append", required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    merge([Path(p) for p in ns.input], Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
