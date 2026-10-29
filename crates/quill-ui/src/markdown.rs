//! Markdown from the server (the rules now; notes and prompts later) as
//! Dioxus elements. Nothing becomes an HTML string: raw HTML in the source
//! shows as text, and links work only for web and mail addresses. The
//! server's own rendered HTML is never used; `docs/plan.md` ("Server text
//! in the desktop webview") says why.

use dioxus::prelude::*;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Paragraph,
    /// h3 to h6: the page's own headings sit above.
    Heading(u8),
    Quote,
    Code,
    List(Option<u64>),
    Item,
    Emphasis,
    Strong,
    Struck,
    /// A link, when its address is safe to open.
    Link(Option<String>),
    /// Anything else whose text is kept (images show their alt text).
    Plain,
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Element(Kind, Vec<Node>),
    Text(String),
    InlineCode(String),
    Break,
    Rule,
}

fn heading(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 3,
        HeadingLevel::H2 => 4,
        HeadingLevel::H3 => 5,
        _ => 6,
    }
}

fn safe_link(dest: &str) -> Option<String> {
    let lower = dest.trim().to_ascii_lowercase();
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|s| lower.starts_with(s))
        .then(|| dest.trim().to_owned())
}

fn kind(tag: Tag<'_>) -> Kind {
    match tag {
        Tag::Paragraph => Kind::Paragraph,
        Tag::Heading { level, .. } => Kind::Heading(heading(level)),
        Tag::BlockQuote(_) => Kind::Quote,
        Tag::CodeBlock(_) => Kind::Code,
        Tag::List(start) => Kind::List(start),
        Tag::Item => Kind::Item,
        Tag::Emphasis => Kind::Emphasis,
        Tag::Strong => Kind::Strong,
        Tag::Strikethrough => Kind::Struck,
        Tag::Link { dest_url, .. } => Kind::Link(safe_link(&dest_url)),
        _ => Kind::Plain,
    }
}

/// The source as a tree of nodes.
fn parse(source: &str) -> Vec<Node> {
    // Each open element: its kind and the children so far.
    let mut stack: Vec<(Kind, Vec<Node>)> = vec![(Kind::Plain, Vec::new())];
    let push = |stack: &mut Vec<(Kind, Vec<Node>)>, node: Node| {
        if let Some((_, children)) = stack.last_mut() {
            children.push(node);
        }
    };
    for event in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(tag) => stack.push((kind(tag), Vec::new())),
            Event::End(_) if stack.len() > 1 => {
                if let Some((k, children)) = stack.pop() {
                    push(&mut stack, Node::Element(k, children));
                }
            }
            Event::End(TagEnd::Paragraph) | Event::End(_) => {}
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                push(&mut stack, Node::Text(t.into_string()));
            }
            Event::Code(t) => push(&mut stack, Node::InlineCode(t.into_string())),
            Event::SoftBreak => push(&mut stack, Node::Text(" ".into())),
            Event::HardBreak => push(&mut stack, Node::Break),
            Event::Rule => push(&mut stack, Node::Rule),
            _ => {}
        }
    }
    stack.into_iter().next().map(|(_, c)| c).unwrap_or_default()
}

fn render(node: &Node) -> Element {
    let kids = |children: &[Node]| children.iter().map(render).collect::<Vec<_>>().into_iter();
    match node {
        Node::Text(t) => rsx! { "{t}" },
        Node::InlineCode(t) => rsx! { code { "{t}" } },
        Node::Break => rsx! { br {} },
        Node::Rule => rsx! { hr {} },
        Node::Element(kind, children) => {
            let inner = kids(children);
            match kind {
                Kind::Paragraph => rsx! { p { {inner} } },
                Kind::Heading(3) => rsx! { h3 { {inner} } },
                Kind::Heading(4) => rsx! { h4 { {inner} } },
                Kind::Heading(5) => rsx! { h5 { {inner} } },
                Kind::Heading(_) => rsx! { h6 { {inner} } },
                Kind::Quote => rsx! { blockquote { {inner} } },
                Kind::Code => rsx! { pre { code { {inner} } } },
                Kind::List(None) => rsx! { ul { {inner} } },
                Kind::List(Some(start)) => rsx! { ol { start: "{start}", {inner} } },
                Kind::Item => rsx! { li { {inner} } },
                Kind::Emphasis => rsx! { em { {inner} } },
                Kind::Strong => rsx! { strong { {inner} } },
                Kind::Struck => rsx! { del { {inner} } },
                Kind::Link(Some(href)) => rsx! {
                    a { href: "{href}", target: "_blank", rel: "noopener noreferrer", {inner} }
                },
                Kind::Link(None) | Kind::Plain => rsx! { span { {inner} } },
            }
        }
    }
}

/// Markdown shown as plain elements.
#[component]
pub fn Markdown(source: String) -> Element {
    let nodes = parse(&source);
    rsx! {
        div { class: "markdown",
            for node in nodes.iter() {
                {render(node)}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(source: &str) -> String {
        dioxus_ssr::render_element(rsx! { Markdown { source: source.to_owned() } })
    }

    #[test]
    fn structure_is_kept_and_headings_sit_under_the_page() {
        let out = html("# Be kind\n\nNo **spam**, *please*.\n\n1. one\n2. two\n\n> quoted");
        assert!(out.contains("<h3>Be kind</h3>"), "{out}");
        assert!(out.contains("<strong>spam</strong>"));
        assert!(out.contains("<em>please</em>"));
        assert!(out.contains(r#"<ol start="1"><li>one</li>"#), "{out}");
        assert!(out.contains("<blockquote>"));
    }

    #[test]
    fn raw_html_is_only_text() {
        let out =
            html("Hi <script>alert(1)</script> <b>bold</b>\n\n<div onclick=\"x\">block</div>");
        assert!(!out.contains("<script"), "{out}");
        assert!(!out.contains("<b>"), "{out}");
        assert!(!out.contains("<div onclick"), "{out}");
        assert!(out.contains("&#60;script&#62;"), "{out}");
    }

    #[test]
    fn only_web_and_mail_links_open() {
        let out = html(
            "[site](https://writers.example/rules) [bad](javascript:alert(1)) [file](file:///etc/passwd)",
        );
        assert!(
            out.contains(r#"href="https://writers.example/rules""#),
            "{out}"
        );
        assert!(!out.contains("javascript:"), "{out}");
        assert!(!out.contains("file:"), "{out}");
        assert!(out.contains("<span>bad</span>"));
    }
}
