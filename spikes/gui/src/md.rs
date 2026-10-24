//! Markdown for the spike: rendering notes (sanitized, as the real client
//! must), styling the source in place for the live-preview editor, and
//! turning the rich editor's HTML back into Markdown.

use pulldown_cmark::{Options, Parser, html};

/// A note's Markdown as safe HTML: raw HTML is escaped by the sanitizer,
/// and images are left out (attachments are shown separately).
pub fn render(markdown: &str) -> String {
    let mut out = String::new();
    let parser = Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH);
    html::push_html(&mut out, parser);
    ammonia::Builder::default()
        .rm_tags(["img"])
        .link_rel(Some("noopener noreferrer nofollow"))
        .clean(&out)
        .to_string()
}

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// Inline marks in one line: `**strong**`, `*em*` / `_em_`, `` `code` ``
/// and `[links](url)`. Every source character is kept, markers included,
/// so the styled text lines up with the text area drawn over it.
fn inline(line: &str, out: &mut String) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut String| {
        escape(plain, out);
        plain.clear();
    };
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        let span = |class: &str, open: usize, body_end: usize, close: usize, out: &mut String| {
            let text: String = chars[i..i + open].iter().collect();
            out.push_str("<span class=\"mk\">");
            escape(&text, out);
            out.push_str("</span><span class=\"");
            out.push_str(class);
            out.push_str("\">");
            let body: String = chars[i + open..body_end].iter().collect();
            escape(&body, out);
            out.push_str("</span><span class=\"mk\">");
            let end: String = chars[body_end..body_end + close].iter().collect();
            escape(&end, out);
            out.push_str("</span>");
        };
        let find = |from: usize, pat: &str| -> Option<usize> {
            let tail: String = chars[from..].iter().collect();
            tail.find(pat).map(|b| from + tail[..b].chars().count())
        };
        if rest.starts_with("**")
            && let Some(end) = find(i + 2, "**").filter(|e| *e > i + 2)
        {
            flush(&mut plain, out);
            span("strong", 2, end, 2, out);
            i = end + 2;
            continue;
        }
        if rest.starts_with('`')
            && let Some(end) = find(i + 1, "`").filter(|e| *e > i + 1)
        {
            flush(&mut plain, out);
            span("code", 1, end, 1, out);
            i = end + 1;
            continue;
        }
        if (rest.starts_with('*') || rest.starts_with('_'))
            && chars.get(i + 1).is_some_and(|c| !c.is_whitespace())
        {
            let m = chars[i].to_string();
            if let Some(end) = find(i + 1, &m).filter(|e| *e > i + 1) {
                flush(&mut plain, out);
                span("em", 1, end, 1, out);
                i = end + 1;
                continue;
            }
        }
        if rest.starts_with('[')
            && let Some(mid) = find(i + 1, "](")
            && let Some(end) = find(mid + 2, ")")
        {
            flush(&mut plain, out);
            span("link", 1, mid, end + 1 - mid, out);
            i = end + 1;
            continue;
        }
        plain.push(chars[i]);
        i += 1;
    }
    flush(&mut plain, out);
}

#[cfg(test)]
/// The Markdown source as HTML with the same characters, styled: drawn
/// behind a transparent text area, it shows formatting as you type while
/// typing, spellcheck and input methods stay the platform's own.
pub fn highlight(source: &str) -> String {
    let mut out = highlight_lines(source)
        .into_iter()
        .map(|l| if l == " " { String::new() } else { l })
        .collect::<Vec<_>>()
        .join("\n");
    // A trailing newline needs a character after it to take up a line.
    out.push(' ');
    out
}

/// The styled source, one string per line without its newline: the view
/// draws each line as its own block, so typing re-lays out one line rather
/// than the whole note (a long note is otherwise far too slow). An empty
/// line holds a space so it keeps its height.
pub fn highlight_lines(source: &str) -> Vec<String> {
    let mut fenced = false;
    source
        .split('\n')
        .map(|line| {
            let mut out = String::with_capacity(line.len() + 16);
            style_line(line, &mut fenced, &mut out);
            if out.is_empty() {
                out.push(' ');
            }
            out
        })
        .collect()
}

fn style_line(line: &str, fenced: &mut bool, out: &mut String) {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        *fenced = !*fenced;
        out.push_str("<span class=\"mk\">");
        escape(line, out);
        out.push_str("</span>");
        return;
    }
    if *fenced {
        out.push_str("<span class=\"code\">");
        escape(line, out);
        out.push_str("</span>");
        return;
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        let marker = line.len() - trimmed.len() + hashes;
        out.push_str("<span class=\"heading\"><span class=\"mk\">");
        escape(&line[..marker], out);
        out.push_str("</span>");
        inline(&line[marker..], out);
        out.push_str("</span>");
        return;
    }
    if trimmed.starts_with('>') {
        let marker = line.len() - trimmed.len() + 1;
        out.push_str("<span class=\"quote\"><span class=\"mk\">");
        escape(&line[..marker], out);
        out.push_str("</span>");
        inline(&line[marker..], out);
        out.push_str("</span>");
        return;
    }
    inline(line, out);
}

/// The rich editor's HTML (what `contenteditable` produced) as Markdown.
/// Handles the few tags the editor makes; anything else keeps its text.
pub fn from_html(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut list_depth = 0usize;
    while !rest.is_empty() {
        if let Some(tag_start) = rest.find('<') {
            text(&rest[..tag_start], &mut out);
            let Some(tag_end) = rest[tag_start..].find('>') else {
                break;
            };
            let tag = &rest[tag_start + 1..tag_start + tag_end];
            rest = &rest[tag_start + tag_end + 1..];
            let closing = tag.starts_with('/');
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            match (name.as_str(), closing) {
                ("b" | "strong", _) => out.push_str("**"),
                ("i" | "em", _) => out.push('*'),
                ("code", _) => out.push('`'),
                ("h1", false) => out.push_str("# "),
                ("h2", false) => out.push_str("## "),
                ("h3", false) => out.push_str("### "),
                ("blockquote", false) => out.push_str("> "),
                ("ul" | "ol", false) => list_depth += 1,
                ("ul" | "ol", true) => list_depth = list_depth.saturating_sub(1),
                ("li", false) => out.push_str("- "),
                ("br", _) => out.push('\n'),
                ("p" | "div" | "h1" | "h2" | "h3" | "blockquote" | "li", true) => {
                    out.push_str("\n\n")
                }
                _ => {}
            }
        } else {
            text(rest, &mut out);
            break;
        }
    }
    let _ = list_depth;
    let trimmed = out.trim_end().to_owned();
    // Runs of blank lines collapse to one.
    let mut tidy = String::with_capacity(trimmed.len());
    let mut blank = 0;
    for line in trimmed.split('\n') {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        tidy.push_str(line);
        tidy.push('\n');
    }
    tidy.trim_end().to_owned()
}

fn text(s: &str, out: &mut String) {
    out.push_str(
        &s.replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&amp;", "&"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_tags(html: &str) -> String {
        let mut out = String::new();
        let mut inside = false;
        for c in html.chars() {
            match c {
                '<' => inside = true,
                '>' if inside => inside = false,
                c if !inside => out.push(c),
                _ => {}
            }
        }
        out.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
    }

    #[test]
    fn highlighting_keeps_every_character() {
        for src in &[
            "# The ledger\n\nIt was **still** wet, *wasn't* it? `ink` and [a map](http://x).\n> said the keeper\n```\ncode <b>\n```\n",
            "**unclosed and * stray _ marks",
            "émoji 🎃 and 漢字 *mixed*",
            "",
            &crate::sample::long_draft(),
        ] {
            let src: &str = src;
            let h = highlight(src);
            assert_eq!(strip_tags(&h), format!("{src} "), "{src:?}");
        }
    }

    #[test]
    fn notes_are_sanitized() {
        let html =
            render("hi <script>alert(1)</script> ![x](http://t/x.png) [l](javascript:alert(1))");
        assert!(!html.contains("<script"));
        assert!(!html.contains("<img"));
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn rich_html_comes_back_as_markdown() {
        assert_eq!(
            from_html(
                "<h2>Title</h2><div>It was <b>wet</b> and <i>cold</i>.</div><div><br></div><blockquote>said</blockquote>"
            ),
            "## Title\n\nIt was **wet** and *cold*.\n\n> said"
        );
    }
}
