"""Plait brand geometry and palette, shared by every generator in build.py.

Everything here is plain math that writes SVG strings, so the art can be
regenerated (or retuned) without a drawing program.
"""

from __future__ import annotations

import math

# ---- Palette -------------------------------------------------------------
# The accent teal and the ink/paper neutrals are the client's existing
# tokens (crates/quill-ui/src/style.css), so the art sits in the app as is.
# The other three are the "thread" colours: one per writer in the art.

LIGHT = {
    "paper": "#eef1f4",
    "surface": "#ffffff",
    "ink": "#1c2430",
    "muted": "#5f6a7c",
    "line": "#d9dee5",
    "teal": "#0e6c78",   # --accent
    "ochre": "#c27f1d",
    "madder": "#b0405a",
    "indigo": "#3d4f96",
}

DARK = {
    "paper": "#12161c",
    "surface": "#1a2029",
    "ink": "#e4e8ee",
    "muted": "#9aa6b8",
    "line": "#2c3440",
    "teal": "#5cc3cc",   # --accent (dark)
    "ochre": "#e8b25e",
    "madder": "#e37a92",
    "indigo": "#8f9fe6",
}

STRANDS = ("teal", "ochre", "madder")


def fmt(v: float) -> str:
    """Short number for SVG output."""
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def smooth_path(points: list[tuple[float, float]]) -> str:
    """A Catmull-Rom spline through the points, as cubic Beziers."""
    if len(points) < 2:
        return ""
    d = [f"M{fmt(points[0][0])} {fmt(points[0][1])}"]
    pts = [points[0], *points, points[-1]]
    for i in range(1, len(pts) - 2):
        p0, p1, p2, p3 = pts[i - 1], pts[i], pts[i + 1], pts[i + 2]
        c1 = (p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6)
        c2 = (p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6)
        d.append(
            f"C{fmt(c1[0])} {fmt(c1[1])} {fmt(c2[0])} {fmt(c2[1])} {fmt(p2[0])} {fmt(p2[1])}"
        )
    return "".join(d)


# ---- The plait -----------------------------------------------------------
# Three strands are sinusoids 120 degrees apart. Where two cross, the one
# heading toward the centre goes over: that is the rule of a real
# three-strand plait (the outer strand is laid over the middle one).


class Plait:
    """Geometry for an n-strand plait running along y (or x if `horizontal`)."""

    def __init__(
        self,
        start: float,
        end: float,
        centre: float,
        amplitude: float,
        wavelength: float,
        strands: int = 3,
        lead_in: float = 0.0,
        lead_spread: float | None = None,
        horizontal: bool = False,
        phase: float = 0.0,
    ):
        self.start, self.end = start, end
        self.centre, self.amp, self.wl = centre, amplitude, wavelength
        self.n = strands
        self.lead_in = lead_in
        self.lead_spread = lead_spread
        self.horizontal = horizontal
        self.phase = phase

    def _theta(self, t: float) -> float:
        # Straight for the lead-in, then the phase advances at a steady rate.
        u = max(0.0, t - (self.start + self.lead_in))
        return self.phase + 2 * math.pi * u / self.wl

    def _offset(self, i: int, t: float) -> float:
        phi = 2 * math.pi * i / self.n
        braided = self.amp * math.sin(self._theta(t) + phi)
        if self.lead_in <= 0 or self.lead_spread is None:
            return braided
        # In the lead-in the strands come in parallel and further apart,
        # easing into the plait: separate threads joining.
        k = (t - self.start) / self.lead_in
        if k >= 1:
            return braided
        e = 0.5 - 0.5 * math.cos(math.pi * max(0.0, k))
        loose = self.lead_spread * math.sin(self.phase + phi)
        return loose + (braided - loose) * e

    def point(self, i: int, t: float) -> tuple[float, float]:
        off = self.centre + self._offset(i, t)
        return (t, off) if self.horizontal else (off, t)

    def strand_points(self, i: int, steps_per_wave: int = 24) -> list[tuple[float, float]]:
        length = self.end - self.start
        steps = max(8, int(steps_per_wave * length / self.wl))
        return [self.point(i, self.start + length * s / steps) for s in range(steps + 1)]

    def crossings(self) -> list[tuple[int, int, float]]:
        """(over, under, t) for every crossing, found by sampling."""
        out = []
        steps = int(400 * (self.end - self.start) / self.wl)
        for a in range(self.n):
            for b in range(a + 1, self.n):
                prev = None
                for s in range(steps + 1):
                    t = self.start + (self.end - self.start) * s / steps
                    diff = self._offset(a, t) - self._offset(b, t)
                    if prev is not None and (diff == 0 or (diff > 0) != (prev[1] > 0)):
                        # Linear interpolation of the crossing.
                        t0, d0 = prev
                        tc = t0 + (t - t0) * d0 / (d0 - diff) if d0 != diff else t
                        out.append(self._who_is_over(a, b, tc))
                    prev = (t, diff)
        return out

    def _who_is_over(self, a: int, b: int, t: float) -> tuple[int, int, float]:
        h = 1e-3 * self.wl
        x = self._offset(a, t)
        da = self._offset(a, t + h) - self._offset(a, t - h)
        # Heading toward the centre means moving against the sign of x.
        a_inward = da * x < 0
        return (a, b, t) if a_inward else (b, a, t)


def plait_svg(
    p: Plait,
    colours: list[str],
    width: float,
    gap: float,
    ids: str,
    canvas: tuple[float, float, float, float],
    cap: str = "round",
    opacity: float = 1.0,
) -> tuple[str, str]:
    """(defs, body) drawing the plait with real gaps at every under-crossing.

    Gaps are masks rather than background-coloured outlines, so the mark
    works on any background, transparent included.
    """
    x0, y0, w, h = canvas
    paths = [smooth_path(p.strand_points(i)) for i in range(p.n)]
    crossings = p.crossings()
    defs, body = [], []
    span = width * 1.6  # how much of the over-strand cuts the under-strand
    for i in range(p.n):
        cuts = []
        for over, under, t in crossings:
            if under != i:
                continue
            seg = [p.point(over, t + d) for d in _frange(-span, span, 9)]
            cuts.append(
                f'<path d="{smooth_path(seg)}" stroke="#000" stroke-width="{fmt(width + 2 * gap)}" '
                f'stroke-linecap="butt" fill="none"/>'
            )
        mid = f"{ids}-m{i}"
        defs.append(
            f'<mask id="{mid}" maskUnits="userSpaceOnUse" x="{fmt(x0)}" y="{fmt(y0)}" '
            f'width="{fmt(w)}" height="{fmt(h)}">'
            f'<rect x="{fmt(x0)}" y="{fmt(y0)}" width="{fmt(w)}" height="{fmt(h)}" fill="#fff"/>'
            + "".join(cuts)
            + "</mask>"
        )
        op = "" if opacity == 1 else f' opacity="{fmt(opacity)}"'
        body.append(
            f'<path d="{paths[i]}" fill="none" stroke="{colours[i % len(colours)]}" '
            f'stroke-width="{fmt(width)}" stroke-linecap="{cap}" mask="url(#{mid})"{op}/>'
        )
    return "".join(defs), "".join(body)


def _frange(a: float, b: float, n: int) -> list[float]:
    return [a + (b - a) * k / (n - 1) for k in range(n)]
