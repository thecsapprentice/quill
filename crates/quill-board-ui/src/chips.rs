//! Tag chips and their colours.
//!
//! The board's CSP allows no inline styles, so chip colours live in a
//! stylesheet generated from the server's settings ([`tag_color_css`]),
//! served from the board's own origin. Chips carry two classes: one for
//! their tag type and one for the tag itself, so a tag's own colour (written
//! later in the sheet) wins over its type's.

use std::fmt::Write as _;

use dioxus::prelude::*;
use quill_proto::{
    board::{ChipColor, Tag, TagNamespace, TagStatus},
    color,
};

/// Where the generated sheet is served.
pub const TAG_COLORS_PATH: &str = "/static/tag-colors.css";

/// Only `[a-z0-9-]` reaches a class name or the stylesheet. Namespaces and
/// slugs are validated to that already; this is a second guard.
fn safe_ident(s: &str) -> Option<&str> {
    (!s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    .then_some(s)
}

/// Class for a tag type's colour.
pub fn type_class(namespace: &str) -> Option<String> {
    safe_ident(namespace).map(|ns| format!("tcn-{ns}"))
}

/// Class for a tag's own colour. `__` can't occur in either part.
pub fn tag_class(namespace: &str, slug: &str) -> Option<String> {
    Some(format!(
        "tc-{}__{}",
        safe_ident(namespace)?,
        safe_ident(slug)?
    ))
}

/// All classes for a chip showing `tag`.
pub fn chip_classes(tag: &Tag) -> String {
    let mut classes = String::from("chip");
    for c in [
        type_class(&tag.namespace),
        tag_class(&tag.namespace, &tag.slug),
    ]
    .into_iter()
    .flatten()
    {
        classes.push(' ');
        classes.push_str(&c);
    }
    if tag.status != TagStatus::Visible {
        classes.push_str(" chip-quiet");
    }
    classes
}

fn rule(out: &mut String, class: &str, c: &ChipColor) {
    // Only validated colours are written; anything else is skipped.
    let Ok(bg) = color::normalize(&c.background) else {
        return;
    };
    let Ok(fg) = color::normalize(&c.text_or_auto()) else {
        return;
    };
    let _ = writeln!(
        out,
        ".chip.{class}{{background:{bg};color:{fg};border-color:{bg}}}"
    );
}

/// The generated stylesheet: tag types first, then tags with their own
/// colours, so a tag's own rule wins (same specificity, later in the sheet).
/// `tags` should hold only visible tags, so the sheet reveals nothing.
pub fn tag_color_css(namespaces: &[TagNamespace], tags: &[(String, String, ChipColor)]) -> String {
    let mut out = String::from("/* Tag chip colours, generated from Server settings. */\n");
    for ns in namespaces {
        if let (Some(c), Some(class)) = (&ns.color, type_class(&ns.name)) {
            rule(&mut out, &class, c);
        }
    }
    for (ns, slug, c) in tags {
        if let Some(class) = tag_class(ns, slug) {
            rule(&mut out, &class, c);
        }
    }
    out
}

/// A chip that isn't a link or a control, such as a preview.
pub fn chip(tag: &Tag) -> Element {
    rsx! {
        span { class: "{chip_classes(tag)}",
            span { class: "tag-ns", "{tag.namespace}: " }
            "{tag.display}"
        }
    }
}

#[cfg(test)]
mod tests {
    use quill_proto::board::TagOrigin;

    use super::*;

    #[test]
    fn classes_are_safe() {
        assert_eq!(
            type_class("content-warning").as_deref(),
            Some("tcn-content-warning")
        );
        assert_eq!(
            tag_class("genre", "slice-of-life").as_deref(),
            Some("tc-genre__slice-of-life")
        );
        assert_eq!(type_class("a{b}"), None);
        assert_eq!(tag_class("genre", "x y"), None);
        let tag = Tag {
            namespace: "genre".into(),
            slug: "horror".into(),
            display: "Horror".into(),
            origin: TagOrigin::Server,
            status: TagStatus::Pending,
            color: None,
        };
        assert_eq!(
            chip_classes(&tag),
            "chip tcn-genre tc-genre__horror chip-quiet"
        );
    }

    #[test]
    fn stylesheet_order_and_escaping() {
        let ns = |name: &str, bg: &str, fg: Option<&str>| TagNamespace {
            name: name.into(),
            display: name.into(),
            locked: false,
            required: false,
            color: Some(ChipColor {
                background: bg.into(),
                text: fg.map(str::to_owned),
            }),
        };
        let css = tag_color_css(
            &[
                ns("genre", "#dbeafe", Some("#1e3a8a")),
                ns("dark", "#1e3a8a", None),
                ns("bad", "red;}body{display:none", None),
                ns("x}y", "#ffffff", None),
            ],
            &[(
                "genre".into(),
                "horror".into(),
                ChipColor {
                    background: "#7f1d1d".into(),
                    text: None,
                },
            )],
        );
        assert!(
            css.contains(".chip.tcn-genre{background:#dbeafe;color:#1e3a8a;border-color:#dbeafe}")
        );
        // Automatic text on a dark background is white.
        assert!(css.contains(".chip.tcn-dark{background:#1e3a8a;color:#ffffff;"));
        assert!(css.find("tcn-genre").unwrap() < css.find("tc-genre__horror").unwrap());
        assert!(!css.contains("display:none"));
        assert!(!css.contains("x}y"));
        assert_eq!(css.matches('{').count(), 3);
    }
}
