//! Renders prompt Markdown to HTML that is safe to show on the board and to
//! federate.
//!
//! Defense in depth: raw HTML in the source is escaped while parsing, and the
//! result is then passed through an allowlist sanitizer. Images become plain
//! links so viewing a prompt never makes the reader's browser contact a
//! third-party host.

use std::collections::HashSet;
use std::sync::LazyLock;

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

static SANITIZER: LazyLock<ammonia::Builder<'static>> = LazyLock::new(|| {
    let tags: HashSet<&str> = [
        "p",
        "br",
        "em",
        "strong",
        "del",
        "blockquote",
        "ul",
        "ol",
        "li",
        "code",
        "pre",
        "hr",
        "a",
        "h3",
        "h4",
        "h5",
        "h6",
    ]
    .into_iter()
    .collect();
    let mut builder = ammonia::Builder::empty();
    builder
        .tags(tags)
        .add_tag_attributes("a", ["href"])
        .add_tag_attributes("ol", ["start"])
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .link_rel(Some("nofollow noopener noreferrer ugc"))
        .strip_comments(true);
    builder
});

/// Converts prompt Markdown to sanitized HTML.
pub fn render(markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH);
    let events = parser.map(|event| match event {
        // Raw HTML is shown as text, never interpreted.
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        // Page-level headings would dominate the board; start at h3.
        Event::Start(Tag::Heading {
            level,
            id,
            classes,
            attrs,
        }) => Event::Start(Tag::Heading {
            level: demote(level),
            id,
            classes,
            attrs,
        }),
        Event::End(TagEnd::Heading(level)) => Event::End(TagEnd::Heading(demote(level))),
        // Images become links to the image, loaded only if the reader clicks.
        Event::Start(Tag::Image {
            dest_url, title, ..
        }) => Event::Start(Tag::Link {
            link_type: pulldown_cmark::LinkType::Inline,
            dest_url,
            title,
            id: CowStr::Borrowed(""),
        }),
        Event::End(TagEnd::Image) => Event::End(TagEnd::Link),
        other => other,
    });
    let mut unsafe_html = String::with_capacity(markdown.len() * 3 / 2);
    html::push_html(&mut unsafe_html, events);
    SANITIZER.clean(&unsafe_html).to_string()
}

/// The text of prompt Markdown without its formatting, with block
/// boundaries as newlines. Link and image destinations are left out; see
/// [`link_destinations`].
pub fn plain_text(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    for event in Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => {
                out.push_str(&t)
            }
            Event::SoftBreak | Event::HardBreak | Event::Rule => out.push('\n'),
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock,
            ) => out.push('\n'),
            _ => {}
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// The destinations of Markdown links and images, in order.
pub fn link_destinations(markdown: &str) -> Vec<String> {
    Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH)
        .filter_map(|event| match event {
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                Some(dest_url.into_string())
            }
            _ => None,
        })
        .collect()
}

fn demote(level: HeadingLevel) -> HeadingLevel {
    match level {
        HeadingLevel::H1 | HeadingLevel::H2 => HeadingLevel::H3,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{link_destinations, plain_text, render};

    #[test]
    fn plain_text_drops_formatting() {
        let text =
            plain_text("# Title\n\nA *keeper* and [a link](https://x.example).\n\n- one\n- two");
        assert_eq!(text, "Title\nA keeper and a link.\none\ntwo");
    }

    #[test]
    fn link_destinations_include_images() {
        let links =
            link_destinations("[a](https://a.example) ![b](https://b.example/i.png) bare.example");
        assert_eq!(links, ["https://a.example", "https://b.example/i.png"]);
    }

    #[test]
    fn basic_formatting() {
        let html = render("A *keeper* finds **entries**.\n\n- one\n- two");
        assert!(html.contains("<em>keeper</em>"));
        assert!(html.contains("<strong>entries</strong>"));
        assert!(html.contains("<li>one</li>"));
    }

    #[test]
    fn raw_html_and_scripts_are_neutralized() {
        let html = render("<script>alert(1)</script>\n\n<b onclick=\"x()\">hi</b>");
        // Raw HTML survives only as escaped, visible text.
        assert!(!html.contains("<script"));
        assert!(!html.contains("<b"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&lt;b onclick="));
    }

    #[test]
    fn links_are_hardened_and_bad_schemes_dropped() {
        let html = render("[site](https://example.com) [x](javascript:alert(1))");
        assert!(html.contains(r#"href="https://example.com""#));
        assert!(html.contains("noopener"));
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn images_become_links_and_headings_are_demoted() {
        let html = render("# Title\n\n![map](https://img.example/map.png)");
        assert!(!html.contains("<img"));
        assert!(!html.contains("<h1"));
        assert!(html.contains("<h3>Title</h3>"));
        assert!(html.contains(r#"href="https://img.example/map.png""#));
        assert!(html.contains("map"));
    }
}
