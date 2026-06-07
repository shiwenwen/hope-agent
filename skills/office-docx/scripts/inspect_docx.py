#!/usr/bin/env python3
"""Inspect and verify DOCX packages."""

from __future__ import annotations

import argparse
import json
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


REQUIRED = {
    "[Content_Types].xml",
    "_rels/.rels",
    "word/document.xml",
    "word/styles.xml",
}


def count_tags(root: ET.Element, suffix: str) -> int:
    return sum(1 for node in root.iter() if node.tag.endswith(suffix))


def inspect(path: Path) -> dict:
    with zipfile.ZipFile(path) as zf:
        names = set(zf.namelist())
        missing = sorted(REQUIRED - names)
        text = []
        paragraph_count = 0
        table_count = 0
        real_list_count = 0
        comment_ref_count = 0
        inserted_count = 0
        deleted_count = 0
        drawing_count = 0
        image_alt_count = 0
        missing_alt_count = 0
        if "word/document.xml" in names:
            root = ET.fromstring(zf.read("word/document.xml"))
            paragraph_count = count_tags(root, "}p")
            table_count = count_tags(root, "}tbl")
            real_list_count = count_tags(root, "}numPr")
            comment_ref_count = count_tags(root, "}commentReference")
            inserted_count = count_tags(root, "}ins")
            deleted_count = count_tags(root, "}del")
            drawing_count = count_tags(root, "}drawing")
            for node in root.iter():
                if node.tag.endswith("}docPr"):
                    descr = node.attrib.get("descr", "")
                    if descr.strip():
                        image_alt_count += 1
                    else:
                        missing_alt_count += 1
            for node in root.iter():
                if node.tag.endswith("}t") and node.text:
                    text.append(node.text)
                elif node.tag.endswith("}delText") and node.text:
                    text.append(node.text)
        media = sorted(name for name in names if name.startswith("word/media/"))
        comments = []
        if "word/comments.xml" in names:
            comment_root = ET.fromstring(zf.read("word/comments.xml"))
            for comment in comment_root:
                comment_text = []
                for node in comment.iter():
                    if node.tag.endswith("}t") and node.text:
                        comment_text.append(node.text)
                comments.append("".join(comment_text))
        has_numbering = "word/numbering.xml" in names
        return {
            "path": str(path),
            "valid_package": not missing,
            "missing": missing,
            "entries": len(names),
            "paragraph_count": paragraph_count,
            "table_count": table_count,
            "real_list_count": real_list_count,
            "has_numbering": has_numbering,
            "comment_count": len(comments),
            "comment_reference_count": comment_ref_count,
            "tracked_insertions": inserted_count,
            "tracked_deletions": deleted_count,
            "image_count": len(media),
            "drawing_count": drawing_count,
            "image_alt_count": image_alt_count,
            "missing_alt_count": missing_alt_count,
            "comments_preview": comments[:20],
            "text_preview": "\n".join(text)[:4000],
        }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("path")
    parser.add_argument("--verify", action="store_true")
    ns = parser.parse_args()
    result = inspect(Path(ns.path))
    print(json.dumps(result, ensure_ascii=False, indent=2))
    if ns.verify and not result["valid_package"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
