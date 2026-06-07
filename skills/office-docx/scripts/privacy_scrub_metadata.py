#!/usr/bin/env python3
"""Scrub common creator/modified metadata from DOCX package properties."""

from __future__ import annotations

import argparse
import re
import zipfile
from pathlib import Path


def scrub_xml(xml: str) -> str:
    replacements = {
        "dc:creator": "Hope Agent",
        "cp:lastModifiedBy": "Hope Agent",
        "cp:revision": "1",
        "Company": "",
        "Manager": "",
    }
    for tag, value in replacements.items():
        xml = re.sub(rf"<{tag}>.*?</{tag}>", f"<{tag}>{value}</{tag}>", xml, flags=re.DOTALL)
    return xml


def scrub_docx(src: Path, out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    targets = {"docProps/core.xml", "docProps/app.xml", "docProps/custom.xml"}
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if item.filename in targets:
                data = scrub_xml(data.decode("utf-8")).encode("utf-8")
            zout.writestr(item, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    scrub_docx(Path(ns.input), Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
