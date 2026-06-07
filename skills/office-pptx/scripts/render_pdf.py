#!/usr/bin/env python3
"""Render PPTX to PDF using LibreOffice/soffice when available."""

from __future__ import annotations

import argparse
import shutil
import subprocess
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("path")
    parser.add_argument("--out-dir", default=None)
    ns = parser.parse_args()
    binary = shutil.which("soffice") or shutil.which("libreoffice")
    if not binary:
        raise SystemExit("LibreOffice/soffice not found in PATH")
    src = Path(ns.path).resolve()
    out_dir = Path(ns.out_dir).resolve() if ns.out_dir else src.parent
    out_dir.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [binary, "--headless", "--convert-to", "pdf", "--outdir", str(out_dir), str(src)],
        check=True,
    )
    print(out_dir / f"{src.stem}.pdf")


if __name__ == "__main__":
    main()
