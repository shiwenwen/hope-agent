#!/usr/bin/env python3
"""Recalculate an XLSX workbook through LibreOffice when available."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("--out", required=True)
    ns = parser.parse_args()

    office = shutil.which("soffice") or shutil.which("libreoffice")
    if not office:
        raise SystemExit("LibreOffice/soffice not found in PATH")

    src = Path(ns.input).resolve()
    out = Path(ns.out).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="xlsx-recalc-") as tmp_raw:
        tmp = Path(tmp_raw)
        work = tmp / src.name
        shutil.copy2(src, work)
        result = subprocess.run(
            [
                office,
                "--headless",
                "--convert-to",
                "xlsx",
                "--outdir",
                str(tmp),
                str(work),
            ],
            text=True,
            capture_output=True,
            check=True,
        )
        produced = tmp / src.name
        if not produced.exists():
            candidates = sorted(tmp.glob("*.xlsx"))
            if not candidates:
                raise SystemExit("LibreOffice did not produce an xlsx file")
            produced = candidates[0]
        shutil.copy2(produced, out)
    print(json.dumps({"input": str(src), "out": str(out), "engine": office}, indent=2))


if __name__ == "__main__":
    main()
