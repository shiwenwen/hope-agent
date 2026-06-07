#!/usr/bin/env python3
"""Redact common sensitive text patterns inside DOCX XML text nodes."""

from __future__ import annotations

import argparse
import re
import zipfile
from pathlib import Path


EMAIL_RE = re.compile(r"[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}")
PHONE_RE = re.compile(r"(?<!\d)(?:\+?1[-.\s]?)?(?:\(?\d{3}\)?[-.\s]?)\d{3}[-.\s]?\d{4}(?!\d)")


def redact_text(text: str, emails: bool, phones: bool, terms: list[str]) -> str:
    if emails:
        text = EMAIL_RE.sub("[REDACTED_EMAIL]", text)
    if phones:
        text = PHONE_RE.sub("[REDACTED_PHONE]", text)
    for term in terms:
        if term:
            text = text.replace(term, "[REDACTED]")
    return text


def redact_xml_text_nodes(xml: str, emails: bool, phones: bool, terms: list[str]) -> str:
    def replace(match: re.Match[str]) -> str:
        open_tag, value, close_tag = match.groups()
        return open_tag + redact_text(value, emails, phones, terms) + close_tag

    return re.sub(
        r"(<(?:w:(?:t|delText)|dc:title)\b[^>]*>)(.*?)(</(?:w:(?:t|delText)|dc:title)>)",
        replace,
        xml,
        flags=re.DOTALL,
    )


def redact_docx(src: Path, out: Path, emails: bool, phones: bool, terms: list[str]) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    xml_parts = {"word/document.xml", "word/comments.xml", "docProps/core.xml"}
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if item.filename in xml_parts:
                data = redact_xml_text_nodes(data.decode("utf-8"), emails, phones, terms).encode("utf-8")
            zout.writestr(item, data)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("out")
    parser.add_argument("--emails", action="store_true")
    parser.add_argument("--phones", action="store_true")
    parser.add_argument("--term", action="append", default=[])
    ns = parser.parse_args()
    redact_docx(Path(ns.input), Path(ns.out), ns.emails, ns.phones, ns.term)
    print(ns.out)


if __name__ == "__main__":
    main()
