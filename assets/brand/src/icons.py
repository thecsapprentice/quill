"""Plait's interface icons: 24px line icons, one path each.

They follow the board's existing nav icons (quill-board-ui layout.rs):
24 by 24, no fill, 2px round strokes in currentColor, so each `d` string
drops straight into that `match`. Names and titles use the interface's own
words (prompt, session, persona); the weaving lives only in the drawings.
"""

from __future__ import annotations

# name: (plain meaning, path data)
ICONS: dict[str, tuple[str, str]] = {
    # ---- Places --------------------------------------------------------
    "home": ("Home", "M4 11l8-7 8 7M6 9.5V20h12V9.5M10 20v-5h4v5"),
    "board": (
        "The prompt board",
        "M3 3h18M7 3v3M17 3v3M3.5 6h7v9h-7zM13.5 6h7v13h-7zM6 10h2M16 10h2",
    ),
    "prompt": (
        "A prompt",
        "M4 3h16M8 3v4M16 3v4M6 7h12v14H6zM9 11h6M9 14h6M9 17h3",
    ),
    "prompt-new": (
        "Post a prompt", "M4 3h16M8 3v4M16 3v4M6 7h12v14H6zM12 10.5v7M8.5 14h7"),
    "my-prompts": (
        "My prompts", "M3 3h14M7 3v3M13 3v3M5 6h10v12H5zM18 9v11H8"),
    "request": (
        "A request to write together",
        "M2 12c3.5-4.5 16.5-4.5 20 0-3.5 4.5-16.5 4.5-20 0zM9 12h6",
    ),
    "request-sent": (
        "A request you sent",
        "M2 15c3-3.5 12-3.5 15 0-3 3.5-12 3.5-15 0zM7.5 15h4.5M15 4h6v6M21 4l-6 6",
    ),
    "request-received": (
        "A request waiting on you",
        "M2 15c3-3.5 12-3.5 15 0-3 3.5-12 3.5-15 0zM7.5 15h4.5M21 4l-6 6M15 5v5h5",
    ),
    "session": (
        "A writing session",
        "M3 4h18M3 20h18M5 4v16M19 4v16M9 4v6M9 14v6M12 4v16M15 4v6M15 14v6M5 12h5.5M13.5 12h5.5",
    ),
    "session-locked": (
        "End-to-end encrypted session",
        "M3 4h18M5 4v16M19 4v7M9 4v7M14 4v7M5 20h5M13 14h8v7h-8zM14.5 14v-1.5a2.5 2.5 0 0 1 5 0V14",
    ),
    # ---- Inside a loom ------------------------------------------------
    "contribution": (
        "One contribution",
        "M3 15c3 0 3-6 6-6s3 6 6 6 3-6 6-6M3 15h.01",
    ),
    "story": (
        "The story channel",
        "M5 5h14v14H5zM5 11l6-6M5 17l12-12M11 19l8-8M17 19l2-2",
    ),
    "planning": (
        "The planning channel",
        "M5 3h14v18H5zM5 9h14M5 15h14M12 3v18M8.5 6h.01M15.5 12h.01M8.5 18h.01",
    ),
    "channels": (
        "All channels", "M3 8l9-4 9 4-9 4zM3 12l9 4 9-4M3 16l9 4 9-4"),
    "export": (
        "Export the finished piece",
        "M6 5a3 7 0 1 0 0 14 3 7 0 1 0 0-14zM6 5h12a3 7 0 0 1 0 14H6M6 10a1 2 0 1 0 0 4",
    ),
    "edit": (
        "Edit",
        "M4 20L15.5 8.5M15.5 8.5l2.5-2.5a1.8 1.8 0 0 1 2.5 2.5L18 11l-2.5-2.5M4 20l1.5-4.5M4 20l4.5-1.5",
    ),
    "attach": (
        "Attach a reference",
        "M20 11.5l-8 8a5 5 0 0 1-7-7l8.5-8.5a3.3 3.3 0 0 1 4.7 4.7L10 17a1.7 1.7 0 0 1-2.4-2.4L15 7.3",
    ),
    # ---- People and trust -------------------------------------------
    "persona": (
        "A persona",
        "M5 4.5a6 1.5 0 1 0 12 0 6 1.5 0 1 0-12 0M5 19.5a6 1.5 0 1 0 12 0 6 1.5 0 1 0-12 0M8 6v12M14 6v12M8 8.5l6 2M8 12l6 2M8 15.5l6 2M14 13c3 0 5 2 6 6",
    ),
    "personas": (
        "All personas",
        "M2 5a4 1.2 0 1 0 8 0 4 1.2 0 1 0-8 0M2 19a4 1.2 0 1 0 8 0 4 1.2 0 1 0-8 0M4 6.2v11.6M8 6.2v11.6M4 9l4 1.5M4 13l4 1.5M14 5a4 1.2 0 1 0 8 0 4 1.2 0 1 0-8 0M14 19a4 1.2 0 1 0 8 0 4 1.2 0 1 0-8 0M16 6.2v11.6M20 6.2v11.6M16 9l4 1.5M16 13l4 1.5",
    ),
    "add-writer": (
        "Add a writer",
        "M2 4.5a5 1.5 0 1 0 10 0 5 1.5 0 1 0-10 0M2 19.5a5 1.5 0 1 0 10 0 5 1.5 0 1 0-10 0M4.5 6v12M9.5 6v12M4.5 9l5 2M4.5 13l5 2M18 9v6M15 12h6",
    ),
    "verified": (
        "Verified identity (safety number)",
        "M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6zM8.5 12l2.5 2.5 4.5-5",
    ),
    "key": ("Identity key", "M3 12a4 4 0 1 0 8 0 4 4 0 1 0-8 0M11 12h10M17 12v3M20 12v2"),
    "devices": (
        "Linked devices",
        "M3 5h11v8H3zM1.5 16.5h14M17 8h5v12h-5zM19.5 17h.01",
    ),
    "backup": ("Backup and recovery", "M3 4h18v4H3zM5 8v12h14V8M12 11v6M9.5 13.5L12 11l2.5 2.5"),
    # ---- The guild (server) -----------------------------------------
    "server": (
        "A server",
        "M3 9l9-5 9 5M4 9h16M6 9v8M10 9v8M14 9v8M18 9v8M4 17h16M3 20h18",
    ),
    "federation": (
        "Other servers (federation)",
        "M3 7a3 3 0 1 0 6 0 3 3 0 1 0-6 0M15 17a3 3 0 1 0 6 0 3 3 0 1 0-6 0M8.5 8.5c3 1.5 1 6 6.5 7.5",
    ),
    "rules": (
        "Server rules",
        "M8 4h11v13a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3v-2h11v2a3 3 0 0 0 3 3M8 4a3 3 0 0 0-3 3v8M11 8h5M11 12h5",
    ),
    "notices": ("Notices", "M6 16v-5a6 6 0 0 1 12 0v5l2 2H4zM10 20a2 2 0 0 0 4 0"),
    "news": (
        "News (announcements)",
        "M4 5h13v14H6a2 2 0 0 1-2-2zM17 9h3v8a2 2 0 0 1-2 2M8 9h5M8 13h5",
    ),
    "tag": ("Tag", "M3 12V4h8l10 10-7 7zM7.5 8h.01"),
    "report": ("Report", "M5 21V4M5 4h11l-2 4 2 4H5"),
    "moderation": (
        "Moderation",
        "M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6zM12 8v4.5M12 16h.01",
    ),
    # ---- Everyday ---------------------------------------------------
    "search": ("Search", "M10.5 17a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13zM15.5 15.5L20 20"),
    "filter": ("Filter", "M4 5h16l-6 7v6l-4 2v-8z"),
    "settings": (
        "Settings",
        "M4 6h9M17 6h3M15 4v4M4 12h3M11 12h9M9 10v4M4 18h11M19 18h1M17 16v4",
    ),
    "waiting": ("Waiting", "M7 3h10M7 21h10M8 3v3l4 6 4-6V3M8 21v-3l4-6 4 6v3"),
    "accept": ("Accept", "M5 12.5l4.5 4.5L19 7.5"),
    "decline": ("Decline", "M6 6l12 12M18 6L6 18"),
    "more": ("More", "M5 12h.01M12 12h.01M19 12h.01"),
    "sign-in": ("Sign in", "M10 17l5-5-5-5M15 12H3M14 4h5v16h-5"),
    "sign-out": ("Sign out", "M15 17l5-5-5-5M20 12H8M10 4H5v16h5"),
}


def svg(name: str, size: int = 24) -> str:
    title, d = ICONS[name]
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 24 24" '
        'fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
        f"<title>{title}</title><path d=\"{d}\"/></svg>"
    )


def sprite() -> str:
    """Every icon as a <symbol>, for <svg><use href="#plait-NAME"/></svg>."""
    syms = "".join(
        f'<symbol id="plait-{n}" viewBox="0 0 24 24"><title>{t}</title><path d="{d}"/></symbol>'
        for n, (t, d) in ICONS.items()
    )
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" style="display:none" fill="none" stroke="currentColor" '
        f'stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{syms}</svg>'
    )


def rust_module() -> str:
    """icons.rs for quill-ui / quill-board-ui: an enum and its path data."""
    def variant(n: str) -> str:
        return "".join(p.capitalize() for p in n.split("-"))

    lines = [
        "//! Plait's line icons, generated by assets/brand/src/build.py: don't edit by hand.",
        "//! 24 by 24, no fill, 2px round strokes in currentColor (see assets/brand/README.md).",
        "",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub enum Icon {",
    ]
    for n, (t, _) in ICONS.items():
        lines.append(f"    /// {t}.")
        lines.append(f"    {variant(n)},")
    lines += ["}", "", "impl Icon {", "    /// The icon's SVG path data.", "    pub const fn path(self) -> &'static str {", "        match self {"]
    for n, (_, d) in ICONS.items():
        lines.append(f'            Icon::{variant(n)} => "{d}",')
    lines += ["        }", "    }", "", "    /// What the icon means, for a title or an accessible name.", "    pub const fn meaning(self) -> &'static str {", "        match self {"]
    for n, (t, _) in ICONS.items():
        lines.append(f'            Icon::{variant(n)} => "{t}",')
    lines += ["        }", "    }", "}", ""]
    return "\n".join(lines)
