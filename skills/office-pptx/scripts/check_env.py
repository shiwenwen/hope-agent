#!/usr/bin/env python3
"""Report required and optional runtime dependencies for office-pptx."""

from __future__ import annotations

import json
import shutil
import sys


def first_available(*names: str) -> str | None:
    for name in names:
        found = shutil.which(name)
        if found:
            return found
    return None


def main() -> None:
    office = first_available("soffice", "libreoffice")
    png_renderer = first_available("pdftoppm", "magick")
    print(
        json.dumps(
            {
                "python": sys.version.split()[0],
                "python_executable": sys.executable,
                "required": {"python3": True},
                "optional": {
                    "libreoffice": bool(office),
                    "libreoffice_path": office,
                    "page_png_renderer": bool(png_renderer),
                    "page_png_renderer_path": png_renderer,
                },
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
