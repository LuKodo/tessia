#!/usr/bin/env python3
"""Regenerates the Tessia logo assets from the opaque source images in ui/assets-src/.

    python3 ui/scripts/make-logo-assets.py            (needs Pillow only)

Inputs (RGB, white background, so NOT transparent):
    ui/assets-src/logo-full.png      icon + wordmark
    ui/assets-src/logo-icon.png      icon only
    ui/assets-src/logo-wordmark.png  wordmark only (kept for reference, not used)

Outputs:
    ui/public/logo.png         full logo, 1000 px wide, transparent background
    ui/public/logo-dark.png    same, wordmark recolored for dark backgrounds
    ui/public/logo-icon.png    icon only, 512 px square
    ui/public/favicon.png      64 px
    ui/public/favicon.ico      16/32/48
    tessia/cmd/tessia/winres/tessia.ico   16..256, input of tessia/scripts/gen-winres.sh
    tessia/cmd/tessia/winres/tessia.icns  only where macOS `iconutil` exists; for a future .app bundle

Background removal: the near-white region connected to the image border becomes transparent (so the
white inside the document stays opaque, which a plain "distance to white" alpha would punch out).
Pixels in a thin band along that region get a soft alpha estimated from the color of the nearby solid
pixels, and their color is un-premultiplied so edges carry no white halo.
"""

import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "ui" / "assets-src"
PUBLIC = ROOT / "ui" / "public"
ICO_DIR = ROOT / "tessia" / "cmd" / "tessia" / "winres"

WHITE_MIN = 244  # a pixel whose darkest channel is >= this counts as background white
BAND = 4  # px of soft edge kept around the background region
SLATE_MAX_SAT = 40  # wordmark pixels with max-min channel spread below this are the dark slate text
DARK_TEXT = (245, 245, 244)  # taupe-100 of tokens.css: readable on the dark surfaces


def white_interiors(white: Image.Image) -> Image.Image:
    """Mask of the white that must stay opaque: the paper of the document icon.

    The page of the icon is white paper inside a gray outline, but a hairline leak connects it to the
    outside, so plain connectivity cannot tell it from the background. Eroding the white mask first
    cuts such narrow leaks; the biggest area that survives (and is not the outside) is the interior.
    The counters of the letters are small and vanish in the erosion, so they become transparent; only the
    biggest surviving area is kept.
    """
    radius = max(2, round(min(white.size) * 0.01))
    core = white.filter(ImageFilter.MinFilter(2 * radius + 1))
    for corner in ((0, 0), (core.width - 1, 0), (0, core.height - 1), (core.width - 1, core.height - 1)):
        if core.getpixel(corner) == 255:
            ImageDraw.floodfill(core, corner, 128)  # the outside
    found = []  # (area, seed) of every other region that survived the erosion
    step = max(2, radius)
    for y in range(0, core.height, step):
        for x in range(0, core.width, step):
            if core.getpixel((x, y)) != 255:
                continue
            before = core.histogram()[255]
            ImageDraw.floodfill(core, (x, y), 60)
            found.append((before - core.histogram()[255], (x, y)))
    if found:  # the page is the biggest one; smaller pockets (gaps between shapes) stay transparent
        ImageDraw.floodfill(core, max(found)[1], 200)
    blob = core.point(lambda v: 255 if v == 200 else 0).filter(ImageFilter.MaxFilter(4 * radius + 1))
    outside = ImageChops.invert(blob)  # flood the outside of the blob; what is left has the holes filled
    ImageDraw.floodfill(outside, (0, 0), 128)
    filled = outside.point(lambda v: 0 if v == 128 else 255)
    return ImageChops.multiply(filled, white)


def cut_background(src: Image.Image) -> Image.Image:
    rgb = src.convert("RGB")
    r, g, b = rgb.split()
    darkest = ImageChops.darker(ImageChops.darker(r, g), b)
    white = darkest.point(lambda v: 255 if v >= WHITE_MIN else 0)
    bg = ImageChops.subtract(white, white_interiors(white))
    near = bg.filter(ImageFilter.MaxFilter(2 * BAND + 1))
    band = ImageChops.subtract(near, bg)

    # Estimate of the solid color behind each edge pixel: darkest value per channel nearby.
    solid = rgb.filter(ImageFilter.MinFilter(4 * BAND + 3))
    alpha = ImageChops.invert(bg)
    out = rgb.copy()
    a_px, o_px, s_px, p_px, b_px = alpha.load(), out.load(), solid.load(), rgb.load(), band.load()
    for y in range(rgb.height):
        for x in range(rgb.width):
            if not b_px[x, y]:
                continue
            p, f = p_px[x, y], s_px[x, y]
            k = max(range(3), key=lambda c: 255 - f[c])
            den = 255 - f[k]
            if den < 20:  # no dark reference nearby: it is the background
                a_px[x, y] = 0
                continue
            a = min(1.0, max(0.0, (255 - p[k]) / den))
            if a < 0.04:
                a_px[x, y] = 0
            elif a < 0.2:
                a_px[x, y] = round(a * 255)
                o_px[x, y] = f
            else:
                a_px[x, y] = round(a * 255)
                o_px[x, y] = tuple(min(255, max(0, round(255 - (255 - v) / a))) for v in p)
    out.putalpha(alpha)
    return out


def crop(im: Image.Image, margin: float = 0.03, square: bool = False) -> Image.Image:
    box = im.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
    im = im.crop(box)
    pad = round(max(im.size) * margin)
    w, h = im.size
    side_w, side_h = (max(w, h), max(w, h)) if square else (w, h)
    canvas = Image.new("RGBA", (side_w + 2 * pad, side_h + 2 * pad), (0, 0, 0, 0))
    canvas.paste(im, (pad + (side_w - w) // 2, pad + (side_h - h) // 2))
    return canvas


def fit_width(im: Image.Image, width: int) -> Image.Image:
    return im.resize((width, round(im.height * width / im.width)), Image.LANCZOS)


def lighten_wordmark(im: Image.Image) -> Image.Image:
    """Recolors the dark slate letters of the wordmark; the orange icon and the dot stay."""
    cols = im.getchannel("A").point(lambda v: 255 if v > 8 else 0).resize((im.width, 1), Image.BOX)
    empty = [cols.getpixel((x, 0)) == 0 for x in range(im.width)]
    # Split at the widest empty gap between the icon and the wordmark.
    best, gap, start = 0, (0, 0), None
    for x in range(im.width + 1):
        if x < im.width and empty[x]:
            start = x if start is None else start
        elif start is not None:
            if start > 0 and x < im.width and x - start > best:  # margins are not the gap
                best, gap = x - start, (start, x)
            start = None
    split = (gap[0] + gap[1]) // 2
    out = im.copy()
    px = out.load()
    for y in range(out.height):
        for x in range(split, out.width):
            r, g, b, a = px[x, y]
            if a and max(r, g, b) - min(r, g, b) < SLATE_MAX_SAT:
                px[x, y] = (*DARK_TEXT, a)
    return out


def save_png(im: Image.Image, path: Path) -> None:
    im.save(path, optimize=True)
    print(f"{path.relative_to(ROOT)}  {im.width}x{im.height}  {path.stat().st_size // 1024} KiB")


def make_icns(icon: Image.Image, path: Path) -> bool:
    if not shutil.which("iconutil"):
        print("iconutil not found: skipping tessia.icns")
        return False
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "tessia.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256):  # each with its @2x; 512 without (1024 px would triple the file)
            icon.resize((size, size), Image.LANCZOS).save(iconset / f"icon_{size}x{size}.png", optimize=True)
            icon.resize((size * 2, size * 2), Image.LANCZOS).save(iconset / f"icon_{size}x{size}@2x.png", optimize=True)
        icon.resize((512, 512), Image.LANCZOS).save(iconset / "icon_512x512.png", optimize=True)
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(path)], check=True)
    return True


def main() -> None:
    PUBLIC.mkdir(parents=True, exist_ok=True)
    ICO_DIR.mkdir(parents=True, exist_ok=True)
    full = crop(cut_background(Image.open(SRC / "logo-full.png")))
    icon = crop(cut_background(Image.open(SRC / "logo-icon.png")), margin=0.02, square=True)

    logo = fit_width(full, 1000)
    save_png(logo, PUBLIC / "logo.png")
    save_png(lighten_wordmark(logo), PUBLIC / "logo-dark.png")
    save_png(icon.resize((512, 512), Image.LANCZOS), PUBLIC / "logo-icon.png")
    save_png(icon.resize((64, 64), Image.LANCZOS), PUBLIC / "favicon.png")

    big = icon.resize((256, 256), Image.LANCZOS)
    big.save(PUBLIC / "favicon.ico", sizes=[(16, 16), (32, 32), (48, 48)])
    big.save(ICO_DIR / "tessia.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
    make_icns(icon, ICO_DIR / "tessia.icns")
    for p in sorted([PUBLIC / "favicon.ico", *ICO_DIR.glob("tessia.ic*")]):
        print(f"{p.relative_to(ROOT)}  {p.stat().st_size // 1024} KiB")


if __name__ == "__main__":
    main()
