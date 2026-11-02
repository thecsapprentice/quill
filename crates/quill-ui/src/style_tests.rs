//! Checks on the stylesheet's tokens (docs/design/gui-design-language.md):
//! contrast in both themes, and the three copies of each theme agreeing.

use std::collections::HashMap;

/// The `--name: #rrggbb;` colours in the block that starts with `selector {`.
fn colours(selector: &str) -> HashMap<String, String> {
    let css = crate::STYLE;
    let start = css
        .find(&format!("{selector} {{"))
        .unwrap_or_else(|| panic!("no {selector} block"));
    let body = &css[start..];
    let body = &body[..body.find('}').unwrap_or(body.len())];
    body.lines()
        .filter_map(|l| {
            let (name, value) = l.trim().strip_prefix("--")?.split_once(':')?;
            let value = value.trim().trim_end_matches(';');
            value
                .starts_with('#')
                .then(|| (name.to_owned(), value.to_owned()))
        })
        .collect()
}

fn luminance(hex: &str) -> f64 {
    let hex = hex.trim_start_matches('#');
    let channel = |i: usize| {
        let c = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0)) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

fn contrast(a: &str, b: &str) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Text colours and what they sit on; control edges and what they sit on.
const TEXT: [(&str, &str); 13] = [
    ("ink", "paper"),
    ("ink", "surface"),
    ("ink", "accent-soft"),
    ("ink", "warn-soft"),
    ("muted", "paper"),
    ("muted", "surface"),
    ("muted", "accent-soft"),
    ("accent", "paper"),
    ("accent", "surface"),
    ("accent-ink", "accent"),
    ("warn", "warn-soft"),
    ("bad", "surface"),
    ("bad-ink", "bad"),
];
const EDGES: [(&str, &str); 4] = [
    ("edge", "paper"),
    ("edge", "surface"),
    ("accent", "paper"),
    ("accent", "surface"),
];

fn check(theme: &str, c: &HashMap<String, String>) {
    for (fg, bg) in TEXT {
        let r = contrast(&c[fg], &c[bg]);
        assert!(
            r >= 4.5,
            "{theme}: --{fg} on --{bg} is {r:.2}:1; text needs 4.5:1"
        );
    }
    for (fg, bg) in EDGES {
        let r = contrast(&c[fg], &c[bg]);
        assert!(
            r >= 3.0,
            "{theme}: --{fg} on --{bg} is {r:.2}:1; a control's edge needs 3:1"
        );
    }
}

#[test]
fn colours_meet_wcag_aa_in_both_themes() {
    check("light", &colours(":root"));
    check("dark", &colours(".quill[data-theme=\"dark\"]"));
}

#[test]
fn each_theme_says_the_same_wherever_it_is_written() {
    // Light is on :root and again for the light setting; dark is under the
    // system's dark setting and again for the dark setting.
    assert_eq!(colours(":root"), colours(".quill[data-theme=\"light\"]"));
    assert_eq!(
        colours("  :root, .quill:not([data-theme=\"light\"])"),
        colours(".quill[data-theme=\"dark\"]")
    );
}

/// Found by hand (#69): `.sr-only` text inside the scrolling `.main` was
/// placed against the window, not `.main`, so it stretched the window and
/// the whole frame slid up past the end of a long page. `.main` must be
/// the containing block for anything positioned inside it.
#[test]
fn hidden_text_stays_inside_the_scrolling_page() {
    let rule = crate::STYLE
        .lines()
        .find(|l| l.starts_with(".main {"))
        .expect("a .main rule");
    assert!(rule.contains("position: relative"), "{rule}");
}
