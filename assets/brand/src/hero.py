"""The login hero: separate threads arriving and being woven into one cloth."""

from __future__ import annotations

from brand import DARK, LIGHT, fmt, smooth_path

W, H = 1600, 1000
FELL = 585          # where loose threads become cloth
CELL_W, CELL_H = 22, 14
THREADS = ("teal", "ochre", "madder", "indigo")


def shade(hex_colour: str, k: float) -> str:
    """Mix toward black (k < 0) or white (k > 0)."""
    r, g, b = (int(hex_colour[i : i + 2], 16) for i in (1, 3, 5))
    t = 255 if k > 0 else 0
    k = abs(k)
    return "#{:02x}{:02x}{:02x}".format(*(round(c + (t - c) * k) for c in (r, g, b)))


def weave_pattern(pid: str, weft: str, warp: str, back: str) -> str:
    """A 2/2 twill tile, 4 warps by 4 picks, anchored at the fell."""
    cells = []
    for j in range(4):
        for i in range(4):
            x, y = i * CELL_W, j * CELL_H
            if (i + j) % 4 in (0, 1):  # weft over warp
                cells.append(
                    f'<rect x="{x + 1}" y="{y + 2}" width="{CELL_W - 2}" height="{CELL_H - 4}" rx="5" fill="{weft}"/>'
                )
            else:  # warp over weft
                cells.append(
                    f'<rect x="{x + 5}" y="{y}" width="{CELL_W - 10}" height="{CELL_H}" rx="4" fill="{warp}"/>'
                )
    return (
        f'<pattern id="{pid}" patternUnits="userSpaceOnUse" x="0" y="{FELL}" '
        f'width="{CELL_W * 4}" height="{CELL_H * 4}">'
        f'<rect width="{CELL_W * 4}" height="{CELL_H * 4}" fill="{back}"/>' + "".join(cells) + "</pattern>"
    )


def bands() -> list[tuple[str, int]]:
    """Stripes of picks, newest first: each writer's turn is a band in their colour."""
    return [
        ("indigo", 2), ("madder", 3), ("teal", 4), ("ochre", 3), ("madder", 2),
        ("teal", 3), ("indigo", 4), ("ochre", 4), ("teal", 2), ("madder", 4),
        ("ochre", 2), ("indigo", 3), ("teal", 4), ("madder", 3),
    ]


def loose_threads(pal: dict) -> list[tuple[str, list[tuple[float, float]]]]:
    """Four writers' threads, arriving from different edges."""
    return [
        ("teal", [(-80, 120), (190, 150), (420, 250), (560, 380), (650, 500), (690, FELL + 40)]),
        ("ochre", [(520, -80), (600, 120), (720, 260), (770, 400), (780, 510), (790, FELL + 40)]),
        ("madder", [(1240, -80), (1110, 120), (960, 250), (900, 390), (885, 510), (890, FELL + 40)]),
    ]


def shuttle(x: float, y: float, pal: dict, wood: str, wood_dark: str) -> str:
    """A boat shuttle lying on the fell, its bobbin showing."""
    body = (
        f'M{x - 120} {y}C{x - 80} {y - 26} {x + 80} {y - 26} {x + 120} {y}'
        f'C{x + 80} {y + 26} {x - 80} {y + 26} {x - 120} {y}Z'
    )
    return (
        f'<g>'
        f'<ellipse cx="{x}" cy="{y + 22}" rx="112" ry="9" fill="#000" opacity="0.10"/>'
        f'<path d="{body}" fill="{wood}"/>'
        f'<path d="M{x - 120} {y}C{x - 80} {y + 26} {x + 80} {y + 26} {x + 120} {y}" fill="none" stroke="{wood_dark}" stroke-width="3"/>'
        f'<rect x="{x - 48}" y="{y - 11}" width="96" height="22" rx="11" fill="{wood_dark}"/>'
        f'<rect x="{x - 40}" y="{y - 8}" width="80" height="16" rx="8" fill="{pal["indigo"]}"/>'
        + "".join(
            f'<line x1="{x - 34 + k * 8}" y1="{y - 7}" x2="{x - 38 + k * 8}" y2="{y + 7}" stroke="{shade(pal["indigo"], -0.3)}" stroke-width="1.5"/>'
            for k in range(10)
        )
        + "</g>"
    )


def hero(pal: dict, dark: bool, ids: str | None = None) -> str:
    """The hero SVG. Ids are prefixed so both themes can be inlined in one page."""
    q = ids or ("hd" if dark else "hl")
    warp = pal["line"] if not dark else "#3a4452"
    warp_hi = shade(warp, 0.35) if not dark else shade(warp, 0.12)
    defs = [
        # Warp threads fade in from the top, leaving calm space up there.
        f'<linearGradient id="{q}-warpfade" x1="0" y1="0" x2="0" y2="1">'
        '<stop offset="0" stop-color="#fff" stop-opacity="0"/>'
        f'<stop offset="0.42" stop-color="#fff" stop-opacity="1"/></linearGradient>'
        f'<mask id="{q}-warpmask"><rect width="{W}" height="{H}" fill="url(#{q}-warpfade)"/></mask>',
        f'<filter id="{q}-soft" x="-5%" y="-5%" width="110%" height="110%">'
        '<feDropShadow dx="0" dy="6" stdDeviation="6" flood-color="#000" flood-opacity="0.16"/></filter>',
    ]
    for t in THREADS:
        # Dark mode mutes the cloth so it doesn't glare against the paper.
        weft = pal[t] if not dark else shade(pal[t], -0.22)
        lift = 0.32 if not dark else 0.06
        drop = -0.35 if not dark else -0.5
        defs.append(weave_pattern(f"{q}-tw-{t}", weft, shade(weft, lift), shade(weft, drop)))

    body = [f'<rect width="{W}" height="{H}" fill="{pal["paper"]}"/>']

    # Warp: taut vertical threads, each a pair of lines for a hint of twist.
    warps = []
    for i in range(W // CELL_W + 1):
        x = i * CELL_W + CELL_W / 2
        warps.append(f'<rect x="{fmt(x - 3)}" y="0" width="6" height="{H}" rx="3" fill="{warp}"/>')
    body.append(f'<g mask="url(#{q}-warpmask)">{"".join(warps)}</g>')

    # Loose threads, in front of the warp, ending into the cloth.
    for name, pts in loose_threads(pal):
        d = smooth_path(pts)
        body.append(
            f'<path d="{d}" fill="none" stroke="{pal[name]}" stroke-width="14" stroke-linecap="round" filter="url(#{q}-soft)"/>'
            f'<path d="{d}" fill="none" stroke="{shade(pal[name], 0.35)}" stroke-width="3" stroke-linecap="round" '
            f'stroke-dasharray="9 7" opacity="0.7"/>'
        )

    # The cloth: bands of twill below the fell.
    y = FELL
    for colour, n in bands():
        h = n * CELL_H
        body.append(f'<rect x="0" y="{y}" width="{W}" height="{h}" fill="url(#{q}-tw-{colour})"/>')
        y += h
    # The cloth recedes toward the bottom edge.
    defs.append(
        f'<linearGradient id="{q}-recede" x1="0" y1="0" x2="0" y2="1">'
        f'<stop offset="0" stop-color="{pal["paper"]}" stop-opacity="0"/>'
        f'<stop offset="1" stop-color="{pal["paper"]}" stop-opacity="0.55"/></linearGradient>'
    )
    body.append(f'<rect x="0" y="{FELL}" width="{W}" height="{H - FELL}" fill="url(#{q}-recede)"/>')
    # The fell itself: a soft shadow where the newest pick is beaten in.
    body.append(
        f'<rect x="0" y="{FELL - 2}" width="{W}" height="4" fill="{shade(pal["paper"], -0.25)}" opacity="0.6"/>'
    )

    wood = "#b98a5a" if not dark else "#9c7550"
    # The shuttle has just thrown the newest (indigo) pick along the fell.
    body.append(
        f'<path d="M1150 {FELL - 30}C1080 {FELL - 22} 1020 {FELL - 6} 960 {FELL + 2}" fill="none" '
        f'stroke="{pal["indigo"]}" stroke-width="8" stroke-linecap="round"/>'
    )
    body.append(shuttle(1270, FELL - 36, pal, wood, shade(wood, -0.3)))

    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" '
        f'preserveAspectRatio="xMidYMid slice" role="img" aria-label="Threads of four colours woven into one cloth">'
        f'<defs>{"".join(defs)}</defs>{"".join(body)}</svg>'
    )


if __name__ == "__main__":
    import sys

    out = sys.argv[1]
    open(f"{out}/hero-light.svg", "w").write(hero(LIGHT, False))
    open(f"{out}/hero-dark.svg", "w").write(hero(DARK, True))
