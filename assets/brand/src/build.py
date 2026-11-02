"""Regenerate every Plait brand asset from these sources.

    python3 assets/brand/src/build.py

Needs Python 3.10+, fontTools and Pillow, and rsvg-convert (librsvg) for
the PNG renders. Outputs land beside this folder; commit them.
"""

from __future__ import annotations

import io
import os
import subprocess
import sys

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import hero as hero_mod  # noqa: E402
import icons  # noqa: E402
import illus  # noqa: E402
import logo  # noqa: E402
from brand import DARK, LIGHT  # noqa: E402

OUT = os.path.dirname(HERE)


def write(rel: str, text: str) -> str:
    path = os.path.join(OUT, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text if text.endswith("\n") else text + "\n")
    return path


def render(svg_path: str, png_rel: str, width: int) -> str:
    out = os.path.join(OUT, png_rel)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    subprocess.run(["rsvg-convert", "-w", str(width), svg_path, "-o", out], check=True)
    return out


def png_bytes(svg_path: str, size: int) -> Image.Image:
    data = subprocess.run(["rsvg-convert", "-w", str(size), svg_path], check=True, capture_output=True).stdout
    return Image.open(io.BytesIO(data)).convert("RGBA")


def main() -> None:
    # Logo, wordmark, lockups.
    for name, make in logo.ICONS.items():
        folder = "app-icon" if name.startswith(("plait-app-icon", "favicon")) else "logo"
        write(f"{folder}/{name}", make())

    # App icon rasters: the small variant below 32px, where crossings blur.
    icon_svg = os.path.join(OUT, "app-icon/plait-app-icon.svg")
    small_svg = os.path.join(OUT, "app-icon/favicon-small.svg")
    for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
        render(small_svg if size < 32 else icon_svg, f"app-icon/png/plait-{size}.png", size)
    frames = [png_bytes(small_svg if s < 32 else icon_svg, s) for s in (16, 24, 32, 48, 64, 128, 256)]
    frames[-1].save(os.path.join(OUT, "app-icon/plait.ico"), sizes=[f.size for f in frames], append_images=frames[:-1])
    # Pillow writes the frames it is given; the largest goes first.
    png_bytes(icon_svg, 48).save(
        os.path.join(OUT, "app-icon/favicon.ico"),
        sizes=[(16, 16), (32, 32), (48, 48)],
        append_images=[png_bytes(small_svg, 16), png_bytes(icon_svg, 32)],
    )
    png_bytes(icon_svg, 1024).save(os.path.join(OUT, "app-icon/plait.icns"))
    render(icon_svg, "app-icon/apple-touch-icon.png", 180)

    # Hero.
    for theme, pal, dark in (("light", LIGHT, False), ("dark", DARK, True)):
        p = write(f"hero/hero-{theme}.svg", hero_mod.hero(pal, dark))
        render(p, f"hero/hero-{theme}.png", 1600)
        render(p, f"hero/hero-{theme}@2x.png", 3200)

    # Interface icons: one file each, a sprite, and the Rust module.
    for name in icons.ICONS:
        write(f"icons/{name}.svg", icons.svg(name))
    write("icons/sprite.svg", icons.sprite())
    write("icons/icons.rs", icons.rust_module())

    # Spot illustrations, loader, band.
    for name, make in illus.SPOTS.items():
        write(f"illustrations/{name}", make())

    # Social cards.
    for theme, pal, dark in (("light", LIGHT, False), ("dark", DARK, True)):
        p = write(f"social/og-card-{theme}.svg", illus.social_card(pal, dark))
        render(p, f"social/og-card-{theme}.png", 1200)


if __name__ == "__main__":
    main()
