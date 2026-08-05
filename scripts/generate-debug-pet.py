#!/usr/bin/env python3
"""Generate the development-only Pet animation diagnostics atlas.

Each used Codex v1 cell has a solid state colour plus exact bilingual state
and zero-based row/frame labels. Contractually unused cells stay transparent.
The committed PNG is deterministic for a given font and lets developers see
both the selected action row and frame cadence.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


CELL_WIDTH = 192
CELL_HEIGHT = 208
FRAME_COUNT = 8
ROWS = [
    ("IDLE", "空闲", (37, 99, 235)),
    ("RUN RIGHT", "向右跑", (5, 150, 105)),
    ("RUN LEFT", "向左跑", (13, 148, 136)),
    ("WAVE", "挥手", (124, 58, 237)),
    ("JUMP", "跳跃", (202, 91, 10)),
    ("FAILED", "失败", (220, 38, 38)),
    ("WAITING", "等待", (79, 70, 229)),
    ("RUNNING", "运行中", (2, 132, 199)),
    ("REVIEW", "审阅", (219, 39, 119)),
]
FRAME_COUNTS = (6, 8, 8, 4, 5, 8, 6, 6, 6)

FONT_CANDIDATES = [
    Path("/System/Library/Fonts/Hiragino Sans GB.ttc"),
    Path("/System/Library/Fonts/AppleSDGothicNeo.ttc"),
    Path("/Library/Fonts/Arial Unicode.ttf"),
    Path("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
    Path("C:/Windows/Fonts/msyh.ttc"),
]


def choose_font(explicit: Path | None) -> Path:
    candidates = [explicit] if explicit else FONT_CANDIDATES
    for candidate in candidates:
        if candidate and candidate.is_file():
            return candidate
    raise SystemExit("No CJK font found; pass --font /path/to/a/CJK-font.ttf")


def shade(colour: tuple[int, int, int], frame: int) -> tuple[int, int, int, int]:
    # Alternating lightness makes frame progression visible even when the
    # small frame counter is difficult to read at the rendered 50% scale.
    factors = (0.82, 0.9, 0.98, 1.06, 0.88, 1.0, 1.1, 0.94)
    factor = factors[frame]
    return tuple(min(255, round(channel * factor)) for channel in colour) + (255,)


def centered_text(
    draw: ImageDraw.ImageDraw,
    y: int,
    text: str,
    font: ImageFont.FreeTypeFont,
    fill: tuple[int, int, int, int],
    cell_x: int,
) -> None:
    box = draw.textbbox((0, 0), text, font=font)
    width = box[2] - box[0]
    draw.text((cell_x + (CELL_WIDTH - width) / 2, y), text, font=font, fill=fill)


def generate(output: Path, font_path: Path) -> None:
    atlas = Image.new("RGBA", (CELL_WIDTH * FRAME_COUNT, CELL_HEIGHT * len(ROWS)))
    draw = ImageDraw.Draw(atlas)
    english_font = ImageFont.truetype(str(font_path), 26)
    chinese_font = ImageFont.truetype(str(font_path), 27)
    frame_font = ImageFont.truetype(str(font_path), 17)
    white = (255, 255, 255, 255)
    muted_white = (255, 255, 255, 210)

    for row, (english, chinese, colour) in enumerate(ROWS):
        for frame in range(FRAME_COUNTS[row]):
            x = frame * CELL_WIDTH
            y = row * CELL_HEIGHT
            draw.rectangle((x, y, x + CELL_WIDTH - 1, y + CELL_HEIGHT - 1), fill=shade(colour, frame))
            centered_text(draw, y + 42, english, english_font, white, x)
            centered_text(draw, y + 82, chinese, chinese_font, white, x)
            centered_text(draw, y + 137, f"ROW {row}  FRAME {frame}", frame_font, muted_white, x)

    output.parent.mkdir(parents=True, exist_ok=True)
    atlas.save(output, format="PNG", optimize=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("src/assets/pets/hope-debug.png"),
    )
    parser.add_argument("--font", type=Path)
    args = parser.parse_args()
    font_path = choose_font(args.font)
    generate(args.output, font_path)
    print(f"generated {args.output} with {font_path}")


if __name__ == "__main__":
    main()
