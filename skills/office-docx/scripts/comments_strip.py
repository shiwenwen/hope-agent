#!/usr/bin/env python3
"""Remove Word comments from a DOCX package."""

from __future__ import annotations

import argparse
import re
import zipfile
from pathlib import Path


def remove_override(content_types: str) -> str:
    return re.sub(
        r'<Override PartName="/word/comments\.xml" ContentType="[^"]+"\s*/>',
        "",
        content_types,
    )


def remove_comments_relationship(rels: str) -> str:
    return re.sub(
        r'<Relationship[^>]+Type="http://schemas\.openxmlformats\.org/officeDocument/2006/relationships/comments"[^>]*/>',
        "",
        rels,
    )


def remove_comment_markup(document: str) -> str:
    document = re.sub(r'<w:commentRangeStart w:id="[0-9]+"\s*/>', "", document)
    document = re.sub(r'<w:commentRangeEnd w:id="[0-9]+"\s*/>', "", document)
    document = re.sub(r'<w:r><w:commentReference w:id="[0-9]+"\s*/></w:r>', "", document)
    return document


def strip_comments(src: Path, out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            if item.filename == "word/comments.xml":
                continue
            data = zin.read(item.filename)
            if item.filename == "word/document.xml":
                data = remove_comment_markup(data.decode("utf-8")).encode("utf-8")
            elif item.filename == "[Content_Types].xml":
                data = remove_override(data.decode("utf-8")).encode("utf-8")
            elif item.filename == "word/_rels/document.xml.rels":
                data = remove_comments_relationship(data.decode("utf-8")).encode("utf-8")
            zout.writestr(item, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    strip_comments(Path(ns.input), Path(ns.out))
    print(ns.out)


if __name__ == "__main__":
    main()
