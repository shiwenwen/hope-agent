#!/usr/bin/env python3
"""Audit or remove simple DOCX header watermarks."""

from __future__ import annotations

import argparse
import json
import zipfile
from pathlib import Path


def audit(path: Path) -> dict:
    with zipfile.ZipFile(path) as zf:
        headers = [name for name in zf.namelist() if name.startswith("word/header") and name.endswith(".xml")]
        texts = []
        for header in headers:
            text = zf.read(header).decode("utf-8")
            texts.append({"part": header, "contains_text": "<w:t>" in text})
    return {"header_count": len(headers), "headers": texts}


def remove(src: Path, out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            if item.filename.startswith("word/header") and item.filename.endswith(".xml"):
                continue
            data = zin.read(item.filename)
            if item.filename == "word/document.xml":
                xml = data.decode("utf-8")
                xml = xml.replace('<w:headerReference w:type="default" r:id="rIdWatermark"/>', "")
                data = xml.encode("utf-8")
            elif item.filename == "word/_rels/document.xml.rels":
                xml = data.decode("utf-8")
                xml = xml.replace('<Relationship Id="rIdWatermark" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>', "")
                data = xml.encode("utf-8")
            zout.writestr(item, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--remove", action="store_true")
    parser.add_argument("--out")
    ns = parser.parse_args()
    if ns.remove:
        if not ns.out:
            raise SystemExit("--out is required with --remove")
        remove(Path(ns.input), Path(ns.out))
        print(ns.out)
    else:
        print(json.dumps(audit(Path(ns.input)), indent=2))


if __name__ == "__main__":
    main()
