"""The plait mark, the app icon and the wordmark lockups."""

from __future__ import annotations

from brand import DARK, LIGHT, Plait, fmt, plait_svg
from wordmark import outline

# One plait for every size: bold, about 1.7 repeats in view.
AMP, WAVE, WIDTH, GAP, PHASE = 110, 340, 70, 18, 0.3


def _strands(pal: dict) -> list[str]:
    return [pal["teal"], pal["ochre"], pal["madder"]]


def app_icon(bg: str, pal: dict, ids: str, radius: float = 112, small: bool = False) -> str:
    """512 square icon: the plait runs off the top and bottom edges.

    `small` draws fewer, fatter crossings, for 16 to 24 pixel renders.
    """
    amp, wave, width, gap = (122, 560, 104, 28) if small else (AMP, WAVE, WIDTH, GAP)
    p = Plait(-60, 572, 256, amp, wave, phase=PHASE + (0.9 if small else 0))
    defs, body = plait_svg(p, _strands(pal), width, gap, ids, (-100, -100, 712, 712), cap="butt")
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">'
        f'<defs>{defs}<clipPath id="{ids}-c"><rect width="512" height="512" rx="{fmt(radius)}"/></clipPath></defs>'
        f'<g clip-path="url(#{ids}-c)"><rect width="512" height="512" fill="{bg}"/>{body}</g></svg>'
    )


def mark_body(
    pal: dict | None, ids: str, x: float = 0, y: float = 0, h: float = 512, length: float = 512
) -> tuple[str, str]:
    """A length of the plait, cut flat, with no background. pal=None draws in currentColor.

    `length` is how much plait shows, in the 512 design's units; `h` is the
    drawn height. A shorter length gives a chunkier mark for small lockups.
    """
    s = h / length
    w = 300 * s
    cx = x + w / 2
    p = Plait(y - 60 * s, y + h + 60 * s, cx, AMP * s, WAVE * s, phase=PHASE)
    cols = _strands(pal) if pal else ["currentColor"] * 3
    defs, body = plait_svg(p, cols, WIDTH * s, GAP * s, ids, (x - 50 * s, y - 100 * s, w + 100 * s, h + 200 * s), cap="butt")
    # Cut flat top and bottom, like a length of braided cord.
    clip = f'<clipPath id="{ids}-c"><rect x="{fmt(x - 20 * s)}" y="{fmt(y)}" width="{fmt(w + 40 * s)}" height="{fmt(h)}"/></clipPath>'
    return defs + clip, f'<g clip-path="url(#{ids}-c)">{body}</g>'


def mark(pal: dict | None, ids: str) -> str:
    defs, body = mark_body(pal, ids)
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 512"><defs>{defs}</defs>{body}</svg>'


def wordmark(colour: str) -> str:
    d, w, asc, desc, xh = outline("plait", 600, 100)
    top, bottom = -asc, desc
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 {fmt(top)} {fmt(w)} {fmt(bottom - top)}">'
        f'<path d="{d}" fill="{colour}"/></svg>'
    )


def lockup(pal: dict | None, text: str, ids: str) -> str:
    """Mark beside the wordmark. The mark spans the ascender-to-descender height."""
    d, w, asc, desc, xh = outline("plait", 600, 100)
    mh = asc + desc
    mdefs, mbody = mark_body(pal, ids, 0, -asc, mh, length=400)
    mw = 300 * mh / 400
    gapx = 0.22 * mw
    total = mw + gapx + w
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 {fmt(-asc)} {fmt(total)} {fmt(mh)}">'
        f'<defs>{mdefs}</defs>{mbody}<path transform="translate({fmt(mw + gapx)} 0)" d="{d}" fill="{text}"/></svg>'
    )


def favicon() -> str:
    """Small sizes: the light-on-ink icon with a smaller corner radius."""
    return app_icon(LIGHT["ink"], DARK, "fav", radius=96)


ICONS = {
    "plait-app-icon.svg": lambda: app_icon(LIGHT["ink"], DARK, "ai"),
    "plait-app-icon-light.svg": lambda: app_icon(LIGHT["paper"], LIGHT, "ail"),
    "plait-app-icon-teal.svg": lambda: app_icon("#0b3f46", DARK, "ait"),
    "plait-mark.svg": lambda: mark(LIGHT, "mk"),
    "plait-mark-dark.svg": lambda: mark(DARK, "mkd"),
    "plait-mark-mono.svg": lambda: mark(None, "mkm"),
    "plait-wordmark.svg": lambda: wordmark(LIGHT["ink"]),
    "plait-wordmark-dark.svg": lambda: wordmark(DARK["ink"]),
    "plait-lockup.svg": lambda: lockup(LIGHT, LIGHT["ink"], "lk"),
    "plait-lockup-dark.svg": lambda: lockup(DARK, DARK["ink"], "lkd"),
    "plait-lockup-mono.svg": lambda: lockup(None, "currentColor", "lkm"),
    "favicon.svg": favicon,
    "favicon-small.svg": lambda: app_icon(LIGHT["ink"], DARK, "fs", radius=96, small=True),
}

if __name__ == "__main__":
    import sys

    for name, make in ICONS.items():
        open(f"{sys.argv[1]}/{name}", "w").write(make())
