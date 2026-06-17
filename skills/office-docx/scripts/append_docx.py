#!/usr/bin/env python3
"""Append structured blocks to an existing DOCX package."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path

from build_docx import BuildContext, block_xml, comments_xml, numbering_xml


def insertion_xml(spec: dict, ctx: BuildContext) -> str:
    blocks = spec.get("blocks") or []
    if not blocks and spec.get("paragraphs"):
        blocks = [{"type": "paragraph", "text": p} for p in spec["paragraphs"]]
    return "".join(block_xml(block, ctx) for block in blocks)


def max_existing_comment_id(document_comments: str | None) -> int:
    if not document_comments:
        return -1
    ids = [int(value) for value in re.findall(r'w:id="([0-9]+)"', document_comments)]
    return max(ids, default=-1)


def ensure_override(content_types: str, part_name: str, content_type: str) -> str:
    if part_name in content_types:
        return content_types
    insert = f'<Override PartName="{part_name}" ContentType="{content_type}"/>'
    return content_types.replace("</Types>", insert + "</Types>")


def ensure_default(content_types: str, extension: str, content_type: str) -> str:
    if f'Extension="{extension}"' in content_types:
        return content_types
    insert = f'<Default Extension="{extension}" ContentType="{content_type}"/>'
    first_override = content_types.find("<Override ")
    if first_override >= 0:
        return content_types[:first_override] + insert + content_types[first_override:]
    return content_types.replace("</Types>", insert + "</Types>")


def ensure_relationship(rels: str, rel_type: str, target: str) -> str:
    if rel_type in rels and f'Target="{target}"' in rels:
        return rels
    existing = [int(value) for value in re.findall(r'Id="rId([0-9]+)"', rels)]
    rel_id = max(existing, default=0) + 1
    insert = f'<Relationship Id="rId{rel_id}" Type="{rel_type}" Target="{target}"/>'
    return rels.replace("</Relationships>", insert + "</Relationships>")


def ensure_relationship_id(rels: str, rel_id: str, rel_type: str, target: str) -> str:
    if f'Id="{rel_id}"' in rels:
        return rels
    insert = f'<Relationship Id="{rel_id}" Type="{rel_type}" Target="{target}"/>'
    return rels.replace("</Relationships>", insert + "</Relationships>")


def merge_comments(existing: str | None, new_xml: str) -> str:
    if not existing:
        return new_xml
    inner_match = re.search(r"<w:comments[^>]*>(.*)</w:comments>", new_xml, flags=re.DOTALL)
    inner = inner_match.group(1) if inner_match else ""
    return existing.replace("</w:comments>", inner + "</w:comments>")


def append_docx(src: Path, spec: dict, out: Path, base_dir: Path | None = None) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(src) as zin:
        names = set(zin.namelist())
        existing_comments = (
            zin.read("word/comments.xml").decode("utf-8") if "word/comments.xml" in names else None
        )
        existing_images = [name for name in names if name.startswith("word/media/image")]
        ctx = BuildContext(
            next_comment_id=max_existing_comment_id(existing_comments) + 1,
            next_image_id=len(existing_images) + 1,
            base_dir=base_dir or Path.cwd(),
        )
        payload = insertion_xml(spec, ctx)
        document = zin.read("word/document.xml").decode("utf-8")
        sect_idx = document.rfind("<w:sectPr")
        if sect_idx >= 0:
            document = document[:sect_idx] + payload + document[sect_idx:]
        else:
            document = document.replace("</w:body>", payload + "</w:body>")

        content_types = zin.read("[Content_Types].xml").decode("utf-8")
        rels = (
            zin.read("word/_rels/document.xml.rels").decode("utf-8")
            if "word/_rels/document.xml.rels" in names
            else '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>'
        )
        content_types = ensure_override(
            content_types,
            "/word/numbering.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
        )
        rels = ensure_relationship(
            rels,
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering",
            "numbering.xml",
        )
        if ctx.comments:
            content_types = ensure_override(
                content_types,
                "/word/comments.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml",
            )
            rels = ensure_relationship(
                rels,
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments",
                "comments.xml",
            )
        for image in ctx.images:
            ext = Path(image.target).suffix.lower().lstrip(".")
            content_types = ensure_default(content_types, ext, image.content_type)
            rels = ensure_relationship_id(
                rels,
                image.rel_id,
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
                image.target,
            )

        with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
            written = set()
            for item in zin.infolist():
                written.add(item.filename)
                if item.filename == "word/document.xml":
                    zout.writestr(item.filename, document)
                elif item.filename == "[Content_Types].xml":
                    zout.writestr(item.filename, content_types)
                elif item.filename == "word/_rels/document.xml.rels":
                    zout.writestr(item.filename, rels)
                elif item.filename == "word/comments.xml" and ctx.comments:
                    zout.writestr(item.filename, merge_comments(existing_comments, comments_xml(ctx.comments)))
                else:
                    zout.writestr(item, zin.read(item.filename))
            if "word/_rels/document.xml.rels" not in written:
                zout.writestr("word/_rels/document.xml.rels", rels)
            if "word/numbering.xml" not in written:
                zout.writestr("word/numbering.xml", numbering_xml())
            if ctx.comments and "word/comments.xml" not in written:
                zout.writestr("word/comments.xml", comments_xml(ctx.comments))
            for image in ctx.images:
                zout.writestr(f"word/{image.target}", image.source.read_bytes())


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, help="Existing .docx path")
    parser.add_argument("--spec", required=True, help="JSON spec with blocks to append")
    parser.add_argument("--out", required=True, help="Output .docx path")
    ns = parser.parse_args()
    spec_path = Path(ns.spec)
    spec = json.loads(spec_path.read_text(encoding="utf-8"))
    append_docx(Path(ns.input), spec, Path(ns.out), spec_path.parent)
    print(ns.out)


if __name__ == "__main__":
    main()
