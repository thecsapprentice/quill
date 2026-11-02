"""Outline the wordmark from Lora (SIL Open Font License) so it needs no font."""

from __future__ import annotations

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

LORA = "/usr/share/fonts/truetype/google-fonts/Lora-Variable.ttf"


def outline(text: str, weight: int = 600, size: float = 100.0, tracking: float = 0.0):
    """(svg path data, advance width, ascender, descender) at `size` px, baseline at y=0."""
    font = instantiateVariableFont(TTFont(LORA), {"wght": weight})
    upm = font["head"].unitsPerEm
    scale = size / upm
    cmap, gs = font.getBestCmap(), font.getGlyphSet()
    hmtx = font["hmtx"]
    pen = SVGPathPen(gs, ntos=lambda v: f"{v:.1f}".rstrip("0").rstrip("."))
    x = 0.0
    for ch in text:
        name = cmap[ord(ch)]
        tp = TransformPen(pen, (scale, 0, 0, -scale, x, 0))
        gs[name].draw(tp)
        x += hmtx[name][0] * scale + tracking
    x -= tracking
    os2 = font["OS/2"]
    return pen.getCommands(), x, os2.sTypoAscender * scale, -os2.sTypoDescender * scale, os2.sxHeight * scale


if __name__ == "__main__":
    d, w, a, de, xh = outline("plait")
    print(w, a, de, xh, len(d))
