#!/usr/bin/env python3
"""Set a basic document protection mode in word/settings.xml."""

from __future__ import annotations

import argparse
import zipfile
from pathlib import Path


def protect(src: Path, out: Path, mode: str) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    tag = f'<w:documentProtection w:edit="{mode}" w:enforcement="1"/>'
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if item.filename == "word/settings.xml":
                xml = data.decode("utf-8")
                xml = xml.replace("</w:settings>", tag + "</w:settings>") if "documentProtection" not in xml else xml
                data = xml.encode("utf-8")
            zout.writestr(item, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--mode", default="readOnly", choices=["readOnly", "comments", "trackedChanges", "forms"])
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    protect(Path(ns.input), Path(ns.out), ns.mode)
    print(ns.out)


if __name__ == "__main__":
    main()
