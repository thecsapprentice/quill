//! The 3a spike: one writing session's view, with made-up content, built
//! with Dioxus for the desktop (`--features desktop`) and the browser
//! (`--features web`). It exists to answer the questions in FINDINGS.md:
//! nothing here talks to a server or holds a key.

mod md;
mod sample;

use std::collections::HashMap;

use base64::Engine as _;
use dioxus::html::HasFileData;
use dioxus::prelude::*;
use sample::{CHANNELS, Image, Note};

const CSS: &str = include_str!("../assets/spike.css");

/// Keeps the caret in view in the live editor: the text area grows with
/// its text inside a scrolling box, which browsers don't scroll for it.
/// Installed once, on the document, so it survives the text area being
/// remounted.
const FOLLOW_JS: &str = r#"
  if (!window.__follow) {
    window.__follow = true;
    const follow = (e) => {
      const t = e.target;
      if (!t || t.id !== 'draft' || !t.classList.contains('over')) return;
      requestAnimationFrame(() => {
        const m = document.querySelector('.mirror');
        const box = document.querySelector('.live');
        if (!m || !box) return;
        const line = t.value.slice(0, t.selectionEnd).split('\n').length - 1;
        const el = m.children[line];
        if (!el) return;
        const top = el.offsetTop, bottom = top + el.offsetHeight;
        if (top < box.scrollTop) box.scrollTop = top;
        else if (bottom > box.scrollTop + box.clientHeight) box.scrollTop = bottom - box.clientHeight + 8;
      });
    };
    for (const kind of ['keyup', 'input', 'click']) document.addEventListener(kind, follow, true);
  }
"#;

/// Records, for each input event in the text area, when the mirror next
/// changed; `__lag()` reports the median and worst delay in milliseconds
/// and how many inputs it saw.
#[cfg(feature = "desktop")]
const MEASURE_JS: &str = r#"
  const pending = [];
  const lags = [];
  document.addEventListener('input', () => pending.push(performance.now()), true);
  new MutationObserver(() => {
    const now = performance.now();
    while (pending.length) lags.push(now - pending.shift());
  }).observe(document.body, { subtree: true, childList: true, characterData: true });
  window.__lag = () => {
    if (!lags.length) return null;
    const s = [...lags].sort((a, b) => a - b);
    return [Math.round(s[s.length >> 1]), Math.round(s[s.length - 1]), s.length];
  };
"#;

fn main() {
    #[cfg(feature = "desktop")]
    {
        use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
        dioxus::LaunchBuilder::desktop()
            .with_cfg(
                Config::new().with_menu(None).with_window(
                    WindowBuilder::new()
                        .with_title("Quill: editor spike")
                        .with_inner_size(LogicalSize::new(1180.0, 780.0)),
                ),
            )
            .launch(App);
    }
    #[cfg(not(feature = "desktop"))]
    dioxus::launch(App);
}

/// The three editor forms 3a compares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// A plain text area, with the rendered note beside it or toggled.
    Preview,
    /// The source styled in place, behind a transparent text area.
    Live,
    /// A `contenteditable` surface, formatted in place.
    Rich,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Preview => "Write + preview",
            Mode::Live => "Live",
            Mode::Rich => "Rich",
        }
    }
}

/// UTF-16 offsets (what the browser reports) to byte offsets in `s`.
fn byte_at(s: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    s.len()
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

#[component]
fn App() -> Element {
    let mut notes = use_signal(sample::notes);
    let mut channel = use_signal(|| "story".to_owned());
    let mut drafts = use_signal(HashMap::<String, String>::new);
    let mut mode = use_signal(|| Mode::Live);
    let mut show_preview = use_signal(|| true);
    let mut focus = use_signal(|| false);
    let mut pending = use_signal(Vec::<Image>::new);
    let mut reply_to = use_signal(|| None::<u32>);
    let mut status = use_signal(String::new);
    // Bumped whenever an editor must start again from the draft: its text
    // was changed by the app, not typed.
    let mut editor_generation = use_signal(|| 0u64);
    let mut dragging = use_signal(|| false);

    // Measurements for FINDINGS.md (QUILL_SPIKE_DEBUG=1): the viewport,
    // and how far the live mirror trails the text area while typing.
    #[cfg(feature = "desktop")]
    use_future(move || async move {
        if std::env::var_os("QUILL_SPIKE_DEBUG").is_none() {
            return;
        }
        let _ = document::eval(MEASURE_JS).await;
        loop {
            if let Ok(v) = document::eval("return [!!window.__follow, (document.querySelector('.live')||{}).scrollTop, (document.querySelector('.live')||{}).scrollHeight, (document.getElementById('draft')||{}).selectionEnd, window.innerWidth, matchMedia('(max-width: 760px)').matches, getComputedStyle(document.querySelector('.channels')).flexDirection, window.__lag ? window.__lag() : null, (() => { const m = document.querySelector('.mirror'), t = document.getElementById('draft'); return m && t ? [m.children.length === t.value.split('\\n').length, t.value.length] : null; })()];").join::<serde_json::Value>().await {
                let rust = drafts.read().values().map(|d| d.encode_utf16().count()).max().unwrap_or(0);
                let done = drafts.read().values().any(|d| d.trim_end().ends_with("keep up!"));
                println!("measure {v} rust-draft-utf16 {rust} typed-all {done}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    });
    use_hook(|| {
        spawn(async move {
            let _ = document::eval(FOLLOW_JS).await;
        });
        // WebKitGTK checks spelling only when asked, unlike WebView2 and
        // WKWebView.
        #[cfg(all(feature = "desktop", target_os = "linux"))]
        {
            use dioxus::desktop::wry::WebViewExtUnix;
            use webkit2gtk::{WebContextExt, WebViewExt};
            if let Some(context) = dioxus::desktop::window().webview.webview().context() {
                context.set_spell_checking_enabled(true);
                let lang = std::env::var("LANG").unwrap_or_default();
                let lang = lang
                    .split('.')
                    .next()
                    .filter(|l| !l.is_empty() && *l != "C")
                    .unwrap_or("en_US");
                context.set_spell_checking_languages(&[lang]);
            }
        }
    });
    let current = channel();
    let draft = drafts.read().get(&current).cloned().unwrap_or_default();
    let words = draft.split_whitespace().count();
    let shown: Vec<Note> = notes
        .read()
        .iter()
        .filter(|n| n.channel == current)
        .cloned()
        .collect();
    let replying = reply_to().and_then(|id| notes.read().iter().find(|n| n.id == id).cloned());

    let mut set_draft = move |text: String| {
        let c = channel();
        drafts.write().insert(c, text);
    };

    let mut send = move || {
        let c = channel();
        let text = drafts.read().get(&c).cloned().unwrap_or_default();
        if text.trim().is_empty() && pending.read().is_empty() {
            return;
        }
        let id = notes.read().iter().map(|n| n.id).max().unwrap_or(0) + 1;
        notes.write().push(Note {
            id,
            author: sample::ME,
            mine: true,
            channel: c.clone(),
            text: text.trim().to_owned(),
            reply_to: reply_to(),
            images: pending.write().drain(..).collect(),
            edited: false,
        });
        drafts.write().remove(&c);
        reply_to.set(None);
        *editor_generation.write() += 1;
        status.set("Sent.".into());
        spawn(async move {
            let _ = document::eval(
                "requestAnimationFrame(() => { const l = document.getElementById('notes'); if (l) l.scrollTop = l.scrollHeight; });",
            )
            .await;
        });
    };

    // Wraps the text area's selection in `marker` (bold, italic, code).
    let wrap = move |marker: &'static str| {
        spawn(async move {
            let Ok(sel) = document::eval(
                "const t = document.getElementById('draft'); return t ? [t.selectionStart, t.selectionEnd] : [0, 0];",
            )
            .join::<(usize, usize)>()
            .await
            else {
                return;
            };
            let c = channel();
            let text = drafts.read().get(&c).cloned().unwrap_or_default();
            let (a, b) = (byte_at(&text, sel.0), byte_at(&text, sel.1));
            let new = format!(
                "{}{marker}{}{marker}{}",
                &text[..a],
                &text[a..b],
                &text[b..]
            );
            let start = sel.0 + utf16_len(marker);
            let end = sel.1 + utf16_len(marker);
            drafts.write().insert(c, new);
            *editor_generation.write() += 1;
            let _ = document::eval(&format!(
                "requestAnimationFrame(() => {{ const t = document.getElementById('draft'); t.focus(); t.setSelectionRange({start}, {end}); }});"
            ))
            .await;
        });
    };

    let on_text_keys = move |e: KeyboardEvent| {
        let m = e.modifiers();
        let command = m.ctrl() || m.meta();
        if command && e.key() == Key::Enter {
            e.prevent_default();
            send();
            return;
        }
        if command && let Key::Character(c) = e.key() {
            let marker = match c.to_lowercase().as_str() {
                "b" => "**",
                "i" => "*",
                "e" => "`",
                _ => return,
            };
            e.prevent_default();
            wrap(marker);
        }
    };

    let add_files = move |files: Vec<dioxus::html::FileData>| {
        spawn(async move {
            for f in files {
                let name = f.name();
                let kind = f.content_type().unwrap_or_else(|| "image/png".into());
                if !kind.starts_with("image/") {
                    status.set(format!(
                        "{name} isn't an image; the spike shows images only."
                    ));
                    continue;
                }
                if let Ok(bytes) = f.read_bytes().await {
                    let url = format!(
                        "data:{kind};base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(&bytes)
                    );
                    pending.write().push(Image { name, url });
                }
            }
        });
    };

    let app_class = if focus() { "app focus" } else { "app" };
    // Only the form in use does any work per keystroke.
    let preview_html = if mode() == Mode::Preview && show_preview() {
        md::render(&draft)
    } else {
        String::new()
    };
    let highlighted = if mode() == Mode::Live {
        md::highlight_lines(&draft)
    } else {
        Vec::new()
    };
    // The rich editor's starting content changes only when it remounts:
    // were it to follow the draft, every keystroke would reset the
    // editor's HTML and its caret.
    let rich_seed = use_memo(move || {
        let _ = editor_generation();
        let c = channel();
        md::render(&drafts.peek().get(&c).cloned().unwrap_or_default())
    });
    let editor_key = format!("{current}-{}", editor_generation());

    rsx! {
        style { {CSS} }
        div { class: "{app_class}",
            header { class: "bar",
                div { class: "title",
                    h1 { "The lighthouse ledger" }
                    span { class: "with", "with {sample::PARTNER}" }
                }
                div { class: "modes", role: "radiogroup", aria_label: "Editor",
                    for m in [Mode::Preview, Mode::Live, Mode::Rich] {
                        button {
                            key: "{m:?}",
                            role: "radio",
                            aria_checked: if mode() == m { "true" } else { "false" },
                            class: if mode() == m { "mode on" } else { "mode" },
                            onclick: move |_| {
                                mode.set(m);
                                *editor_generation.write() += 1;
                            },
                            "{m.label()}"
                        }
                    }
                }
                button {
                    class: "ghost",
                    aria_pressed: if focus() { "true" } else { "false" },
                    onclick: move |_| focus.toggle(),
                    if focus() { "Leave focus" } else { "Focus" }
                }
            }
            nav { class: "channels", aria_label: "Channels",
                for c in CHANNELS.iter() {
                    button {
                        key: "{c.name}",
                        class: if current == c.name { "channel on" } else { "channel" },
                        aria_current: if current == c.name { "page" } else { "false" },
                        title: "{c.about}",
                        onclick: move |_| {
                            channel.set(c.name.to_owned());
                            reply_to.set(None);
                            *editor_generation.write() += 1;
                        },
                        span { class: "hash", "#" }
                        "{c.name}"
                        if drafts.read().get(c.name).is_some_and(|d| !d.trim().is_empty()) {
                            span { class: "dot", title: "Draft", aria_label: "has a draft" }
                        }
                    }
                }
            }
            main { class: "session",
                section {
                    id: "notes",
                    class: "notes",
                    role: "feed",
                    aria_label: "Notes in #{current}",
                    aria_live: "polite",
                    for n in shown {
                        NoteView {
                            key: "{n.id}",
                            note: n.clone(),
                            quoted: n.reply_to.and_then(|id| notes.read().iter().find(|x| x.id == id).map(|x| x.text.clone())),
                            on_reply: move |id| reply_to.set(Some(id)),
                        }
                    }
                }
                section {
                    class: if dragging() { "composer dragging" } else { "composer" },
                    aria_label: "Write",
                    ondragover: move |e| {
                        e.prevent_default();
                        dragging.set(true);
                    },
                    ondragleave: move |_| dragging.set(false),
                    ondrop: move |e: DragEvent| {
                        e.prevent_default();
                        dragging.set(false);
                        add_files(e.files());
                    },
                    if let Some(r) = replying {
                        div { class: "replying",
                            span { "Replying to {r.author}: " }
                            em { {r.text.chars().take(80).collect::<String>()} }
                            button { class: "ghost small", aria_label: "Stop replying", onclick: move |_| reply_to.set(None), "×" }
                        }
                    }
                    match mode() {
                        Mode::Preview => rsx! {
                            div { class: if show_preview() { "split" } else { "split single" },
                                // A list of one, keyed: a new key remounts it (a key on a
                                // lone element is ignored).
                                for k in [editor_key.clone()] {
                                textarea {
                                    id: "draft",
                                    class: "writing plain",
                                    spellcheck: "true",
                                    // No browser writing suggestions (Edge's text
                                    // prediction): they get in the way, and a
                                    // browser's assistant may send what's typed
                                    // to its maker's servers (FINDINGS 9).
                                    "writingsuggestions": "false",
                                    autocomplete: "off",
                                    lang: "en",
                                    aria_label: "Note in #{current}",
                                    placeholder: "Write in #{current}…  (Markdown; Ctrl+Enter sends)",
                                    // Uncontrolled: the text area owns its text while typing (see
                                    // FINDINGS: a controlled one loses keystrokes); Rust only
                                    // replaces it by remounting (a new key).
                                    key: "{k}",
                                    initial_value: "{draft}",
                                    oninput: move |e| set_draft(e.value()),
                                    onkeydown: on_text_keys,
                                }
                                }
                                if show_preview() {
                                    div { class: "preview rendered", aria_label: "Preview", dangerous_inner_html: "{preview_html}" }
                                }
                            }
                        },
                        Mode::Live => rsx! {
                            div { class: "live",
                                div { class: "writing mirror", aria_hidden: "true",
                                    // One element per line: typing updates one line, not the note.
                                    for (i, line) in highlighted.iter().enumerate() {
                                        div { key: "{i}", class: "line", dangerous_inner_html: "{line}" }
                                    }
                                }
                                // A list of one, keyed: a new key remounts it (a key on a
                                // lone element is ignored).
                                for k in [editor_key.clone()] {
                                textarea {
                                    id: "draft",
                                    class: "writing over",
                                    spellcheck: "true",
                                    // No browser writing suggestions (Edge's text
                                    // prediction): they get in the way, and a
                                    // browser's assistant may send what's typed
                                    // to its maker's servers (FINDINGS 9).
                                    "writingsuggestions": "false",
                                    autocomplete: "off",
                                    lang: "en",
                                    aria_label: "Note in #{current}",
                                    placeholder: "Write in #{current}…  (Markdown; Ctrl+Enter sends)",
                                    // Uncontrolled: the text area owns its text while typing (see
                                    // FINDINGS: a controlled one loses keystrokes); Rust only
                                    // replaces it by remounting (a new key).
                                    key: "{k}",
                                    initial_value: "{draft}",
                                    oninput: move |e| set_draft(e.value()),
                                    onkeydown: on_text_keys,
                                }
                                }
                            }
                        },
                        Mode::Rich => rsx! {
                            for k in [editor_key.clone()] {
                            div {
                                key: "{k}",
                                id: "rich",
                                class: "writing rich rendered",
                                contenteditable: "true",
                                spellcheck: "true",
                                "writingsuggestions": "false",
                                lang: "en",
                                role: "textbox",
                                aria_multiline: "true",
                                aria_label: "Note in #{current}",
                                dangerous_inner_html: "{rich_seed}",
                                oninput: move |_| {
                                    spawn(async move {
                                        if let Ok(html) = document::eval("return document.getElementById('rich').innerHTML;").join::<String>().await {
                                            set_draft(md::from_html(&html));
                                        }
                                    });
                                },
                                onkeydown: move |e: KeyboardEvent| {
                                    let m = e.modifiers();
                                    if (m.ctrl() || m.meta()) && e.key() == Key::Enter {
                                        e.prevent_default();
                                        send();
                                    }
                                },
                            }
                            }
                        },
                    }
                    if !pending.read().is_empty() {
                        div { class: "attachments",
                            for (i, img) in pending.read().iter().enumerate() {
                                figure { key: "{i}",
                                    img { src: "{img.url}", alt: "{img.name}" }
                                    figcaption { "{img.name}" }
                                    button { class: "ghost small", aria_label: "Remove {img.name}", onclick: move |_| { pending.write().remove(i); }, "×" }
                                }
                            }
                        }
                    }
                    div { class: "tools",
                        label { class: "ghost file",
                            "Add image"
                            input {
                                r#type: "file",
                                accept: "image/*",
                                multiple: true,
                                onchange: move |e: FormEvent| add_files(e.files()),
                            }
                        }
                        if mode() == Mode::Preview {
                            button { class: "ghost", onclick: move |_| show_preview.toggle(),
                                if show_preview() { "Hide preview" } else { "Show preview" }
                            }
                        }
                        button { class: "ghost", onclick: move |_| { set_draft(sample::long_draft()); *editor_generation.write() += 1; }, "Load a long draft" }
                        div { class: "send-group",
                            span { class: "count", "{words} words" }
                            span { class: "status", role: "status", "{status}" }
                            button { class: "send", onclick: move |_| send(), "Send" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn NoteView(note: Note, quoted: Option<String>, on_reply: EventHandler<u32>) -> Element {
    let html = md::render(&note.text);
    let id = note.id;
    rsx! {
        article { class: if note.mine { "note mine" } else { "note" }, aria_label: "{note.author}",
            header {
                span { class: "author", "{note.author}" }
                if note.edited { span { class: "edited", "edited" } }
                button { class: "ghost small reply", onclick: move |_| on_reply.call(id), "Reply" }
            }
            if let Some(q) = quoted {
                blockquote { class: "quoted", {q.chars().take(120).collect::<String>()} }
            }
            div { class: "rendered", dangerous_inner_html: "{html}" }
            for (i, img) in note.images.iter().enumerate() {
                figure { key: "{i}", class: "inline-image",
                    img { src: "{img.url}", alt: "{img.name}" }
                }
            }
        }
    }
}
