//! The brand's files the board serves as they are: the favicons, the mark
//! in the header, the sign-in hero and the social card. The server mounts
//! each `Asset` at its path; the pages link them from here.

use dioxus::prelude::*;

/// One file served at a fixed path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

const fn svg(path: &'static str, bytes: &'static [u8]) -> Asset {
    Asset {
        path,
        content_type: "image/svg+xml",
        bytes,
    }
}

const fn png(path: &'static str, bytes: &'static [u8]) -> Asset {
    Asset {
        path,
        content_type: "image/png",
        bytes,
    }
}

pub const FAVICON_ICO: Asset = Asset {
    path: "/favicon.ico",
    content_type: "image/x-icon",
    bytes: include_bytes!("../../../assets/brand/app-icon/favicon.ico"),
};
pub const FAVICON_SVG: Asset = svg(
    "/favicon.svg",
    include_bytes!("../../../assets/brand/app-icon/favicon.svg"),
);
pub const APPLE_TOUCH_ICON: Asset = png(
    "/apple-touch-icon.png",
    include_bytes!("../../../assets/brand/app-icon/apple-touch-icon.png"),
);
pub const MARK_LIGHT: Asset = svg(
    "/static/brand/plait-mark.svg",
    include_bytes!("../../../assets/brand/logo/plait-mark.svg"),
);
pub const MARK_DARK: Asset = svg(
    "/static/brand/plait-mark-dark.svg",
    include_bytes!("../../../assets/brand/logo/plait-mark-dark.svg"),
);
pub const HERO_LIGHT: Asset = svg(
    "/static/brand/hero-light.svg",
    crate::art::HERO_LIGHT.as_bytes(),
);
pub const HERO_DARK: Asset = svg(
    "/static/brand/hero-dark.svg",
    crate::art::HERO_DARK.as_bytes(),
);
/// The link-preview card. Only a light one: previews show on the sharing
/// site, whose theme the board can't know.
pub const SOCIAL_CARD: Asset = png(
    "/static/brand/social-card.png",
    include_bytes!("../../../assets/brand/social/og-card-light.png"),
);

pub const ALL: [Asset; 8] = [
    FAVICON_ICO,
    FAVICON_SVG,
    APPLE_TOUCH_ICON,
    MARK_LIGHT,
    MARK_DARK,
    HERO_LIGHT,
    HERO_DARK,
    SOCIAL_CARD,
];

/// The browser caches these for a week. Their URLs carry no version, so a
/// changed file reaches visitors within that week rather than at once.
pub const CACHE_CONTROL: &str = "public, max-age=604800";

/// A light drawing with its dark-theme counterpart, decorative (empty
/// `alt`), switched by the system theme as the rest of the board is.
fn themed(light: Asset, dark: Asset, class: &'static str) -> Element {
    rsx! {
        picture { class,
            source { "srcset": dark.path, "media": "(prefers-color-scheme: dark)" }
            img { src: light.path, alt: "" }
        }
    }
}

/// The Plait mark, beside the server's name in the header.
pub(crate) fn mark() -> Element {
    themed(MARK_LIGHT, MARK_DARK, "site-mark")
}

/// The woven hero above the sign-in and register forms.
pub(crate) fn hero() -> Element {
    themed(HERO_LIGHT, HERO_DARK, "auth-hero")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_asset_has_its_own_path_and_looks_like_its_type() {
        let mut paths: Vec<_> = ALL.iter().map(|a| a.path).collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), ALL.len());
        for a in ALL {
            let starts: &[u8] = match a.content_type {
                "image/svg+xml" => b"<svg ",
                "image/png" => b"\x89PNG",
                "image/x-icon" => b"\0\0\x01\0",
                other => panic!("{other}"),
            };
            assert!(a.bytes.starts_with(starts), "{}", a.path);
        }
    }

    #[test]
    fn the_served_svgs_have_no_scripts_or_inline_styles() {
        // Inline styles would also break under the board's CSP if inlined.
        for a in ALL.iter().filter(|a| a.content_type == "image/svg+xml") {
            let text = std::str::from_utf8(a.bytes).unwrap();
            for bad in [
                "<script",
                "onload",
                "href=\"http",
                "<foreignObject",
                "style=",
            ] {
                assert!(!text.contains(bad), "{} has {bad}", a.path);
            }
        }
    }
}
