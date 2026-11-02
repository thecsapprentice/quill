"""Spot illustrations, the loading shuttle, the plait band and the social card.

Colours are CSS variables with fallbacks: inlined into the app they follow
its light and dark tokens; opened on their own they use the light palette.
"""

from __future__ import annotations

from brand import DARK, LIGHT, Plait, fmt, plait_svg, smooth_path
from hero import hero
from logo import lockup
from wordmark import outline

# Token references. --thread-* are new tokens (see the README's style.css block).
INK = "var(--muted, #5f6a7c)"
LINE = "var(--line, #d9dee5)"
SURFACE = "var(--surface, #ffffff)"
TEAL = "var(--thread-teal, #0e6c78)"
OCHRE = "var(--thread-ochre, #c27f1d)"
MADDER = "var(--thread-madder, #b0405a)"
INDIGO = "var(--thread-indigo, #3d4f96)"
WOOD = "var(--wood, #b98a5a)"


def _svg(body: str, label: str, w: int = 240, h: int = 160) -> str:
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" '
        f'role="img" aria-label="{label}" fill="none" stroke-linecap="round" stroke-linejoin="round">{body}</svg>'
    )


def beam(x1: float, x2: float, y: float) -> str:
    return f'<path d="M{x1} {y}H{x2}" stroke="{INK}" stroke-width="6"/>'


def empty_board() -> str:
    """Bare warp threads hanging from a beam, and a dashed space for a first warp."""
    parts = [beam(30, 210, 30)]
    lengths = [70, 96, 58, 110, 84, 64, 100]
    for k, ln in enumerate(lengths):
        x = 48 + k * 24
        parts.append(f'<path d="M{x} 30V{30 + ln}" stroke="{LINE}" stroke-width="3"/>')
        parts.append(f'<circle cx="{x}" cy="{30 + ln + 5}" r="3" fill="{LINE}"/>')
    parts.append(
        f'<rect x="92" y="54" width="56" height="70" rx="6" fill="{SURFACE}" stroke="{TEAL}" '
        'stroke-width="2.5" stroke-dasharray="6 6"/>'
    )
    parts.append(f'<path d="M120 76v26M107 89h26" stroke="{TEAL}" stroke-width="3"/>')
    return _svg("".join(parts), "No prompts yet")


def shuttle_shape(x: float, y: float, length: float, bobbin: str) -> str:
    h = length * 0.22
    half = length / 2
    body = (
        f"M{fmt(x - half)} {fmt(y)}C{fmt(x - half * 0.6)} {fmt(y - h)} {fmt(x + half * 0.6)} {fmt(y - h)} {fmt(x + half)} {fmt(y)}"
        f"C{fmt(x + half * 0.6)} {fmt(y + h)} {fmt(x - half * 0.6)} {fmt(y + h)} {fmt(x - half)} {fmt(y)}Z"
    )
    bw = length * 0.4
    return (
        f'<path d="{body}" fill="{WOOD}" stroke="{INK}" stroke-width="2"/>'
        f'<rect x="{fmt(x - bw / 2)}" y="{fmt(y - h * 0.38)}" width="{fmt(bw)}" height="{fmt(h * 0.76)}" '
        f'rx="{fmt(h * 0.38)}" fill="{bobbin}"/>'
    )


def empty_requests() -> str:
    """A shuttle at rest on the beam, its thread trailing loose."""
    tail = smooth_path([(150, 98), (176, 104), (188, 122), (172, 136), (150, 128), (140, 140)])
    parts = [
        f'<path d="M24 112H216" stroke="{LINE}" stroke-width="4"/>',
        f'<path d="{tail}" stroke="{OCHRE}" stroke-width="3"/>',
        shuttle_shape(110, 96, 120, OCHRE),
    ]
    return _svg("".join(parts), "No requests yet")


def empty_sessions() -> str:
    """A loom strung with warp and no weft yet; a spool waits beside it."""
    parts = [
        f'<rect x="40" y="20" width="120" height="124" rx="4" stroke="{INK}" stroke-width="5"/>',
    ]
    for k in range(9):
        x = 54 + k * 11.5
        parts.append(f'<path d="M{fmt(x)} 24V140" stroke="{LINE}" stroke-width="2.5"/>')
    # Two picks woven at the bottom: where the first session will start.
    for row, colour in ((128, TEAL), (120, MADDER)):
        d = []
        for k in range(9):
            x = 54 + k * 11.5
            if (k + (row // 8)) % 2 == 0:
                d.append(f"M{fmt(x - 5.75)} {row}H{fmt(x + 5.75)}")
        parts.append(f'<path d="{"".join(d)}" stroke="{colour}" stroke-width="5" stroke-linecap="butt"/>')
    # The spool.
    parts += [
        f'<ellipse cx="196" cy="104" rx="16" ry="5" fill="{SURFACE}" stroke="{INK}" stroke-width="2.5"/>',
        f'<rect x="186" y="104" width="20" height="34" fill="{INDIGO}"/>',
        f'<ellipse cx="196" cy="138" rx="16" ry="5" fill="{SURFACE}" stroke="{INK}" stroke-width="2.5"/>',
        f'<path d="M186 112l20 6M186 122l20 6" stroke="{SURFACE}" stroke-width="1.5" opacity="0.6"/>',
        f'<path d="M206 120c8 4 10 14 4 22" stroke="{INDIGO}" stroke-width="2.5"/>',
    ]
    return _svg("".join(parts), "No sessions yet")


def no_results() -> str:
    """A loose thread that curls into a question."""
    q = smooth_path([(20, 130), (70, 132), (96, 120), (104, 98), (98, 66), (120, 40), (150, 44), (162, 66), (152, 88), (132, 98), (130, 116)])
    parts = [
        f'<path d="{q}" stroke="{MADDER}" stroke-width="5"/>',
        f'<circle cx="130" cy="136" r="5" fill="{MADDER}"/>',
        f'<path d="M20 146H220" stroke="{LINE}" stroke-width="3"/>',
    ]
    return _svg("".join(parts), "Nothing found")


def offline() -> str:
    """A thread snapped between two guilds."""
    left = smooth_path([(20, 80), (60, 70), (90, 86), (108, 78)])
    right = smooth_path([(132, 84), (150, 74), (180, 92), (220, 80)])
    parts = [
        f'<path d="{left}" stroke="{TEAL}" stroke-width="5"/>',
        f'<path d="{right}" stroke="{TEAL}" stroke-width="5"/>',
        f'<path d="M108 78l4-6M108 78l5 3M108 78l3 6M132 84l-4-6M132 84l-5 2M132 84l-2 6" stroke="{TEAL}" stroke-width="2"/>',
    ]
    return _svg("".join(parts), "Can't reach the server right now")


def loader() -> str:
    """The shuttle passing back and forth, laying picks. Stops if motion is reduced."""
    warps = "".join(f'<path d="M{24 + k * 16} 8V56" stroke="{LINE}" stroke-width="3"/>' for k in range(9))
    style = (
        "<style>"
        "@keyframes plait-throw{0%,100%{transform:translateX(0)}50%{transform:translateX(120px)}}"
        "@keyframes plait-pick{0%{stroke-dashoffset:144}50%{stroke-dashoffset:0}100%{stroke-dashoffset:-144}}"
        ".plait-s{animation:plait-throw 1.6s ease-in-out infinite}"
        ".plait-p{stroke-dasharray:144;animation:plait-pick 1.6s ease-in-out infinite}"
        "@media (prefers-reduced-motion:reduce){.plait-s,.plait-p{animation:none}.plait-p{stroke-dasharray:none}}"
        "</style>"
    )
    body = (
        style
        + warps
        + f'<path class="plait-p" d="M20 32H164" stroke="{TEAL}" stroke-width="4"/>'
        + f'<g class="plait-s">{shuttle_shape(28, 32, 48, TEAL)}</g>'
    )
    return _svg(body, "Working", 184, 64)


def band(pal: dict | None = None, width: float = 480, height: float = 24, ids: str = "band") -> str:
    """A thin horizontal plait, for the top edge of the welcome card and dividers."""
    cols = [TEAL, OCHRE, MADDER] if pal is None else [pal["teal"], pal["ochre"], pal["madder"]]
    p = Plait(-20, width + 20, height / 2, height * 0.3, height * 2.6, horizontal=True)
    defs, body = plait_svg(p, cols, height * 0.24, height * 0.07, ids, (-40, -10, width + 80, height + 20), cap="butt")
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {fmt(width)} {fmt(height)}" '
        f'preserveAspectRatio="xMidYMid slice" aria-hidden="true"><defs>{defs}</defs>{body}</svg>'
    )


TAGLINE_TEXT = "Prompts in public. Writing together, in private."


def TAGLINE(pal: dict) -> str:  # noqa: N802 - reads like the constant it stands for
    """The tagline as outlines, so the card needs no font."""
    return outline(TAGLINE_TEXT, 400, 34)[0]


def social_card(pal: dict, dark: bool) -> str:
    """1200x630 link preview: the hero, the lockup and the tagline."""
    art = hero(pal, dark)
    inner = art[art.index(">") + 1 : art.rindex("</svg>")]
    lk = lockup(pal, pal["ink"], "og")
    lk_inner = lk[lk.index(">") + 1 : lk.rindex("</svg>")]
    # The lockup's viewBox starts at y = -ascender; place it with a transform.
    vb = [float(v) for v in lk.split('viewBox="')[1].split('"')[0].split()]
    scale = 120 / vb[3]
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 630">'
        f'<svg x="0" y="0" width="1200" height="630" viewBox="200 160 1200 630" preserveAspectRatio="xMidYMid slice">{inner}</svg>'
        '<defs><linearGradient id="og-veil" x1="0" y1="0" x2="0" y2="1">'
        f'<stop offset="0" stop-color="{pal["paper"]}" stop-opacity="0.96"/>'
        f'<stop offset="0.62" stop-color="{pal["paper"]}" stop-opacity="0.9"/>'
        f'<stop offset="1" stop-color="{pal["paper"]}" stop-opacity="0"/></linearGradient></defs>'
        '<rect x="0" y="0" width="1200" height="330" fill="url(#og-veil)"/>'
        f'<g transform="translate(72 {fmt(70 - vb[1] * scale)}) scale({fmt(scale)})">{lk_inner}</g>'
        f'<path transform="translate(76 225)" d="{TAGLINE(pal)}" fill="{pal["muted"]}"/></svg>'
    )


SPOTS = {
    "empty-board.svg": empty_board,
    "empty-requests.svg": empty_requests,
    "empty-sessions.svg": empty_sessions,
    "no-results.svg": no_results,
    "offline.svg": offline,
    "loader-shuttle.svg": loader,
    "plait-band.svg": band,
}
