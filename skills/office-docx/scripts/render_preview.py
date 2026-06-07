#!/usr/bin/env python3
"""Render an Office file to PDF and, when available, page PNG previews."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path


def convert_to_pdf(src: Path, out_dir: Path) -> Path:
    binary = shutil.which("soffice") or shutil.which("libreoffice")
    if not binary:
        raise SystemExit("LibreOffice/soffice not found in PATH")
    result = subprocess.run(
        [binary, "--headless", "--convert-to", "pdf", "--outdir", str(out_dir), str(src)],
        text=True,
        capture_output=True,
        check=True,
    )
    if result.stdout:
        print(result.stdout.strip(), file=sys.stderr)
    if result.stderr:
        print(result.stderr.strip(), file=sys.stderr)
    return out_dir / f"{src.stem}.pdf"


def render_pngs(pdf: Path, out_dir: Path) -> tuple[str | None, list[str]]:
    if shutil.which("pdftoppm"):
        prefix = out_dir / "page"
        subprocess.run(["pdftoppm", "-png", "-r", "144", str(pdf), str(prefix)], check=True, stdout=subprocess.DEVNULL)
        return "pdftoppm", [str(path) for path in sorted(out_dir.glob("page-*.png"))]
    if shutil.which("magick"):
        target = out_dir / "page-%03d.png"
        subprocess.run(["magick", "-density", "144", str(pdf), str(target)], check=True, stdout=subprocess.DEVNULL)
        return "magick", [str(path) for path in sorted(out_dir.glob("page-*.png"))]
    return None, []


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("path")
    parser.add_argument("--out-dir", default=None)
    parser.add_argument("--no-png", action="store_true")
    ns = parser.parse_args()
    src = Path(ns.path).resolve()
    out_dir = Path(ns.out_dir).resolve() if ns.out_dir else src.parent / f"{src.stem}-preview"
    out_dir.mkdir(parents=True, exist_ok=True)
    pdf = convert_to_pdf(src, out_dir)
    renderer = None
    pngs: list[str] = []
    if not ns.no_png:
        renderer, pngs = render_pngs(pdf, out_dir)
    print(json.dumps({"pdf": str(pdf), "png_renderer": renderer, "pngs": pngs}, indent=2))


if __name__ == "__main__":
    main()
