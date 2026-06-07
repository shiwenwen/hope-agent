#!/usr/bin/env python3
"""Apply simple tracked text replacements in DOCX document.xml."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from xml.sax.saxutils import escape


TEXT_RUN_RE = re.compile(
    r"<w:r>(?P<rpr><w:rPr>.*?</w:rPr>)?<w:t(?P<attrs>[^>]*)>(?P<text>.*?)</w:t></w:r>",
    re.DOTALL,
)


def text_run_from_escaped(text: str, rpr: str = "") -> str:
    if not text:
        return ""
    preserve = ' xml:space="preserve"' if text != text.strip() else ""
    return f"<w:r>{rpr}<w:t{preserve}>{text}</w:t></w:r>"


def revision_pair(old: str, new: str, rev_id: int, rpr: str = "") -> str:
    date = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return (
        f'<w:del w:id="{rev_id}" w:author="Hope Agent" w:date="{date}"><w:r>{rpr}<w:delText>{escape(old)}</w:delText></w:r></w:del>'
        f'<w:ins w:id="{rev_id + 1}" w:author="Hope Agent" w:date="{date}"><w:r>{rpr}<w:t>{escape(new)}</w:t></w:r></w:ins>'
    )


def apply_replacements(xml: str, replacements: list[dict]) -> tuple[str, int]:
    count = 0
    rev_id = 1000
    for item in replacements:
        old = str(item.get("old", ""))
        new = str(item.get("new", ""))
        if not old:
            continue
        limit = int(item.get("count", 0) or 0)
        escaped_old = escape(old)
        hits = 0

        def replace_run(match: re.Match[str]) -> str:
            nonlocal hits, rev_id
            if limit and hits >= limit:
                return match.group(0)
            text = match.group("text")
            if escaped_old not in text:
                return match.group(0)

            rpr = match.group("rpr") or ""
            pieces: list[str] = []
            remaining = text
            while escaped_old in remaining and (not limit or hits < limit):
                before, remaining = remaining.split(escaped_old, 1)
                pieces.append(text_run_from_escaped(before, rpr))
                pieces.append(revision_pair(old, new, rev_id, rpr))
                rev_id += 2
                hits += 1
            pieces.append(text_run_from_escaped(remaining, rpr))
            return "".join(pieces)

        xml = TEXT_RUN_RE.sub(replace_run, xml)
        count += hits
        if hits == 0:
            rev_id += 2
    return xml, count


def patch_docx(src: Path, spec: dict, out: Path) -> dict:
    out.parent.mkdir(parents=True, exist_ok=True)
    total = 0
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if item.filename == "word/document.xml":
                xml, total = apply_replacements(data.decode("utf-8"), spec.get("replacements") or [])
                data = xml.encode("utf-8")
            elif item.filename == "word/settings.xml":
                xml = data.decode("utf-8")
                if "<w:trackRevisions" not in xml:
                    xml = xml.replace("</w:settings>", "<w:trackRevisions/></w:settings>")
                data = xml.encode("utf-8")
            zout.writestr(item, data)
    return {"input": str(src), "out": str(out), "tracked_replacements": total}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True)
    parser.add_argument("--spec", required=True)
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()
    spec = json.loads(Path(ns.spec).read_text(encoding="utf-8"))
    print(json.dumps(patch_docx(Path(ns.input), spec, Path(ns.out)), indent=2))


if __name__ == "__main__":
    main()
