#!/usr/bin/env python3
"""Create a lightweight HTML contact sheet from rendered slide PNGs."""

from __future__ import annotations

import argparse
import html
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--images", nargs="+", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--title", default="Presentation Contact Sheet")
    ns = parser.parse_args()
    out = Path(ns.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    cards = []
    for idx, raw in enumerate(ns.images, 1):
        path = Path(raw).resolve()
        cards.append(
            f'<figure><img src="{html.escape(path.as_uri())}" alt="Slide {idx} preview"/>'
            f"<figcaption>Slide {idx}</figcaption></figure>"
        )
    out.write_text(
        """<!doctype html>
<html><head><meta charset="utf-8"/>
<style>
body{font-family:Arial,sans-serif;margin:24px;background:#f8fafc;color:#0f172a}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(240px,1fr));gap:18px}
figure{margin:0;background:white;border:1px solid #dbe3ef;padding:10px}
img{width:100%;height:auto;display:block}
figcaption{font-size:12px;color:#475569;margin-top:8px}
</style></head><body>
"""
        + f"<h1>{html.escape(ns.title)}</h1><div class=\"grid\">"
        + "\n".join(cards)
        + "</div></body></html>\n",
        encoding="utf-8",
    )
    print(out)


if __name__ == "__main__":
    main()
