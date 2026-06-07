#!/usr/bin/env python3
"""Accept or reject simple tracked insertions/deletions in a DOCX package."""

from __future__ import annotations

import argparse
import re
import zipfile
from pathlib import Path


def unwrap_deleted_to_text(match: re.Match[str]) -> str:
    inner = match.group(1)
    inner = re.sub(r"<w:delText([^>]*)>", r"<w:t\1>", inner)
    inner = inner.replace("</w:delText>", "</w:t>")
    return inner


def apply_revision_mode(document: str, mode: str) -> str:
    if mode == "accept":
        document = re.sub(r"<w:del\b[^>]*>.*?</w:del>", "", document, flags=re.DOTALL)
        document = re.sub(r"<w:ins\b[^>]*>(.*?)</w:ins>", r"\1", document, flags=re.DOTALL)
    else:
        document = re.sub(r"<w:ins\b[^>]*>.*?</w:ins>", "", document, flags=re.DOTALL)
        document = re.sub(r"<w:del\b[^>]*>(.*?)</w:del>", unwrap_deleted_to_text, document, flags=re.DOTALL)
    return document


def clear_track_revisions(settings: str) -> str:
    return re.sub(r"<w:trackRevisions\s*/>", "", settings)


def apply_changes(src: Path, out: Path, mode: str) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if item.filename == "word/document.xml":
                data = apply_revision_mode(data.decode("utf-8"), mode).encode("utf-8")
            elif item.filename == "word/settings.xml":
                data = clear_track_revisions(data.decode("utf-8")).encode("utf-8")
            zout.writestr(item, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--mode", choices=["accept", "reject"], default="accept")
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    apply_changes(Path(ns.input), Path(ns.out), ns.mode)
    print(ns.out)


if __name__ == "__main__":
    main()
