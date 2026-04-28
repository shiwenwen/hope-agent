#!/usr/bin/env python3
"""Generate Tauri bundle icons from a source image.

Usage:
    python3 scripts/generate-icons.py <source.png> [-o OUTPUT_DIR]

Produces:
    - Desktop PNGs: 32x32, 64x64, 128x128, 128x128@2x, icon.png (512)
    - Windows Store: Square*Logo.png (9 sizes), StoreLogo.png
    - iOS AppIcon-*: all standard sizes
    - Android mipmap-[m|h|xh|xxh|xxxh]dpi: ic_launcher{,_round,_foreground}.png
    - icon.ico: multi-size Windows ICO bundle
    - icon.icns: macOS, Apple Big Sur template
                 (824x824 squircle content + 100px transparent margin on 1024 canvas)

Non-mac outputs: plain equal-ratio resize of the source. No template/padding
added — bypasses Tauri CLI's auto-masking behavior.

Tray icon (menu.png) is preserved by default; pass --include-menu-icon
to overwrite it with a 64x64 resize of the source.

Requires:
    - Python 3 with Pillow
    - macOS (needed only for `iconutil` → icon.icns). Without it, the iconset
      folder is produced but .icns is skipped.
    - Source image: PNG/JPG/etc., ≥1024x1024 square recommended.
"""
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    from PIL import Image, ImageChops, ImageFilter
except ImportError:
    sys.exit("Pillow required: pip install Pillow")

# ---------- Apple macOS icns template ----------
ICNS_CANVAS = 1024
ICNS_CONTENT = 824  # 100px margin each side per Big Sur template
SQUIRCLE_N = 5.0  # superellipse exponent (approximates "continuous corner")
SQUIRCLE_SUPERSAMPLE = 8

# Professional macOS icon polish. These are deliberately subtle: the artwork
# should still lead, while the shadow and edge treatment keep the icon from
# feeling flat on light/dark Dock backgrounds.
ICNS_SHADOW_LAYERS = [
    # dx, dy, blur, opacity
    (0, 30, 44, 34),  # broad ambient grounding shadow
    (0, 18, 24, 42),  # main soft shadow
    (0, 5, 10, 26),   # contact shadow near the icon edge
]
ICNS_INNER_EDGE_OPACITY = 18
ICNS_TOP_HIGHLIGHT_OPACITY = 16
ICNS_BOTTOM_SHADE_OPACITY = 12

# Apple-prescribed iconset slot sizes (name → pixel size)
ICNS_SLOTS = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
]

DESKTOP_PNGS = {
    "32x32.png": 32,
    "64x64.png": 64,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 512,
}

WINDOWS_STORE = {
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
}

IOS_ICONS = {
    "AppIcon-20x20@1x.png": 20,
    "AppIcon-20x20@2x.png": 40,
    "AppIcon-20x20@2x-1.png": 40,
    "AppIcon-20x20@3x.png": 60,
    "AppIcon-29x29@1x.png": 29,
    "AppIcon-29x29@2x.png": 58,
    "AppIcon-29x29@2x-1.png": 58,
    "AppIcon-29x29@3x.png": 87,
    "AppIcon-40x40@1x.png": 40,
    "AppIcon-40x40@2x.png": 80,
    "AppIcon-40x40@2x-1.png": 80,
    "AppIcon-40x40@3x.png": 120,
    "AppIcon-60x60@2x.png": 120,
    "AppIcon-60x60@3x.png": 180,
    "AppIcon-76x76@1x.png": 76,
    "AppIcon-76x76@2x.png": 152,
    "AppIcon-83.5x83.5@2x.png": 167,
    "AppIcon-512@2x.png": 1024,
}

ANDROID_DPIS = {
    "mipmap-mdpi": 48,
    "mipmap-hdpi": 72,
    "mipmap-xhdpi": 96,
    "mipmap-xxhdpi": 144,
    "mipmap-xxxhdpi": 192,
}
ANDROID_VARIANTS = ["ic_launcher.png", "ic_launcher_round.png", "ic_launcher_foreground.png"]

ICO_SIZES = [(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (24, 24), (16, 16)]

MENU_ICON_SIZE = 64  # macOS tray icon (menu.png), consumed by tray.rs via include_bytes!


def squircle_mask(size: int, n: float = SQUIRCLE_N, ss: int = SQUIRCLE_SUPERSAMPLE) -> Image.Image:
    """Build an anti-aliased superellipse mask (|x|^n + |y|^n <= 1)."""
    big = size * ss
    mask = Image.new("L", (big, big), 0)
    px = mask.load()
    r = big / 2.0
    half = big // 2
    for y in range(half):
        ny = (y + 0.5 - r) / r
        nyn = abs(ny) ** n
        for x in range(half):
            nx = (x + 0.5 - r) / r
            if abs(nx) ** n + nyn <= 1.0:
                px[x, y] = 255
                px[big - 1 - x, y] = 255
                px[x, big - 1 - y] = 255
                px[big - 1 - x, big - 1 - y] = 255
    return mask.resize((size, size), Image.LANCZOS)


def load_source(path: Path) -> Image.Image:
    img = Image.open(path).convert("RGBA")
    w, h = img.size
    if w != h:
        print(f"warning: source is not square ({w}x{h}) — stretching to 1024x1024", file=sys.stderr)
    if min(w, h) < 512:
        print(f"warning: source is small ({w}x{h}); upscaling will be lossy", file=sys.stderr)
    if img.size != (ICNS_CANVAS, ICNS_CANVAS):
        img = img.resize((ICNS_CANVAS, ICNS_CANVAS), Image.LANCZOS)
    return img


def resize_save(src: Image.Image, dest: Path, size: int) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    src.resize((size, size), Image.LANCZOS).save(dest, "PNG", optimize=True)


def build_ico(src: Image.Image, dest: Path) -> None:
    src.save(dest, format="ICO", sizes=ICO_SIZES)


def alpha_mask_on_canvas(mask: Image.Image, x: int, y: int) -> Image.Image:
    canvas_mask = Image.new("L", (ICNS_CANVAS, ICNS_CANVAS), 0)
    canvas_mask.paste(mask, (x, y))
    return canvas_mask


def add_shadow(canvas: Image.Image, mask: Image.Image, offset: int) -> None:
    for dx, dy, blur, opacity in ICNS_SHADOW_LAYERS:
        alpha = alpha_mask_on_canvas(mask, offset + dx, offset + dy)
        alpha = alpha.filter(ImageFilter.GaussianBlur(blur))
        alpha = alpha.point(lambda p, o=opacity: p * o // 255)
        shadow = Image.new("RGBA", (ICNS_CANVAS, ICNS_CANVAS), (0, 0, 0, 255))
        shadow.putalpha(alpha)
        canvas.alpha_composite(shadow)


def clipped_vertical_alpha(size: int, mask: Image.Image, top: int, bottom: int) -> Image.Image:
    alpha = Image.new("L", (size, size), 0)
    px = alpha.load()
    for y in range(size):
        t = y / max(size - 1, 1)
        value = int(top + (bottom - top) * t)
        if value:
            for x in range(size):
                px[x, y] = value
    return ImageChops.multiply(alpha, mask)


def polish_content(content: Image.Image, mask: Image.Image) -> Image.Image:
    polished = content.copy()

    # A very slight top lift and bottom shade give the white tile depth without
    # making the generated source look glossy.
    top_highlight = Image.new("RGBA", content.size, (255, 255, 255, 255))
    top_highlight.putalpha(
        clipped_vertical_alpha(ICNS_CONTENT, mask, ICNS_TOP_HIGHLIGHT_OPACITY, 0)
    )
    polished.alpha_composite(top_highlight)

    bottom_shade = Image.new("RGBA", content.size, (0, 0, 0, 255))
    bottom_shade.putalpha(
        clipped_vertical_alpha(ICNS_CONTENT, mask, 0, ICNS_BOTTOM_SHADE_OPACITY)
    )
    polished.alpha_composite(bottom_shade)

    inner_edge = ImageChops.subtract(mask, mask.filter(ImageFilter.MinFilter(3)))
    inner_edge = inner_edge.point(lambda p: p * ICNS_INNER_EDGE_OPACITY // 255)
    edge = Image.new("RGBA", content.size, (0, 0, 0, 255))
    edge.putalpha(inner_edge)
    polished.alpha_composite(edge)

    return polished


def build_icns_source(src: Image.Image) -> Image.Image:
    """Apple Big Sur template: polished 824 squircle centered in 1024."""
    content = src.resize((ICNS_CONTENT, ICNS_CONTENT), Image.LANCZOS)
    mask = squircle_mask(ICNS_CONTENT)
    r, g, b, a = content.split()
    content.putalpha(ImageChops.multiply(a, mask))
    content = polish_content(content, mask)
    canvas = Image.new("RGBA", (ICNS_CANVAS, ICNS_CANVAS), (0, 0, 0, 0))
    offset = (ICNS_CANVAS - ICNS_CONTENT) // 2
    add_shadow(canvas, mask, offset)
    canvas.paste(content, (offset, offset), content)
    return canvas


def build_icns(src: Image.Image, dest: Path) -> bool:
    icns_src = build_icns_source(src)
    if shutil.which("iconutil") is None:
        print("warning: iconutil not found (macOS-only) — falling back to Tauri CLI", file=sys.stderr)
    else:
        with tempfile.TemporaryDirectory() as tmp:
            iconset = Path(tmp) / "icon.iconset"
            iconset.mkdir()
            for name, size in ICNS_SLOTS:
                icns_src.resize((size, size), Image.LANCZOS).save(iconset / name, "PNG", optimize=True)
            try:
                subprocess.run(
                    ["iconutil", "-c", "icns", str(iconset), "-o", str(dest)],
                    check=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                return True
            except subprocess.CalledProcessError:
                print("warning: iconutil rejected the iconset — falling back to Tauri CLI", file=sys.stderr)

    pnpm = shutil.which("pnpm")
    if pnpm is None:
        print("warning: pnpm not found — skipping icon.icns", file=sys.stderr)
        return False

    with tempfile.TemporaryDirectory() as tmp:
        tmp_dir = Path(tmp)
        template = tmp_dir / "macos-template.png"
        out_dir = tmp_dir / "tauri-icons"
        out_dir.mkdir()
        icns_src.save(template, "PNG")
        try:
            subprocess.run(
                [pnpm, "exec", "tauri", "icon", str(template), "-o", str(out_dir)],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except subprocess.CalledProcessError:
            print("warning: Tauri CLI failed — skipping icon.icns", file=sys.stderr)
            return False
        generated = out_dir / "icon.icns"
        if not generated.is_file():
            print("warning: Tauri CLI did not produce icon.icns", file=sys.stderr)
            return False
        shutil.copy2(generated, dest)
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source", type=Path, help="Source image (square PNG/JPG, ≥1024x1024 recommended)")
    parser.add_argument(
        "-o", "--output",
        type=Path,
        default=Path("src-tauri/icons"),
        help="Output directory (default: src-tauri/icons)",
    )
    parser.add_argument(
        "--include-menu-icon",
        action="store_true",
        help=f"Also regenerate menu.png ({MENU_ICON_SIZE}x{MENU_ICON_SIZE} tray icon). Off by default — the tray icon usually differs from the app icon.",
    )
    args = parser.parse_args()

    if not args.source.is_file():
        return f"error: source not found: {args.source}"
    if not args.output.is_dir():
        return f"error: output directory does not exist: {args.output}"

    print(f"source:  {args.source}")
    print(f"output:  {args.output}\n")

    src = load_source(args.source)

    # Non-mac: plain resize, no template
    print("desktop PNGs...")
    for name, size in DESKTOP_PNGS.items():
        resize_save(src, args.output / name, size)

    print("Windows Store logos...")
    for name, size in WINDOWS_STORE.items():
        resize_save(src, args.output / name, size)

    print("iOS AppIcon-*...")
    for name, size in IOS_ICONS.items():
        resize_save(src, args.output / "ios" / name, size)

    print("Android mipmap-*...")
    for dpi, size in ANDROID_DPIS.items():
        for variant in ANDROID_VARIANTS:
            resize_save(src, args.output / "android" / dpi / variant, size)

    print("icon.ico...")
    build_ico(src, args.output / "icon.ico")

    print("icon.icns (Apple template: 824 squircle + 100px margin)...")
    build_icns(src, args.output / "icon.icns")

    menu = args.output / "menu.png"
    if args.include_menu_icon:
        print(f"menu.png ({MENU_ICON_SIZE}x{MENU_ICON_SIZE})...")
        resize_save(src, menu, MENU_ICON_SIZE)
    else:
        print(f"\nmenu.png: {'preserved (--include-menu-icon to overwrite)' if menu.exists() else 'absent'}")
    print("done.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
