//! Posting and editing a prompt (mockup D3; M3b and N5 when narrow): the
//! writing on the left with a live preview, the tag panel on the right.
//! Each tag type shows its count against its limits, and Post stays off,
//! saying what's missing, until they're met (#52).

use dioxus::prelude::*;
use quill_board_ui::{maturity_label, state_label};
use quill_proto::board::{
    CreatePrompt, Maturity, Prompt, PromptState, Tag, TagNamespace, TagOrigin, TagPageQuery,
    TagPolicy, TagQuery, TagStatus, UpdatePrompt, limit_label,
};
use uuid::Uuid;

use super::{BoardState, View, reader::Chip, use_board, use_board_epoch, use_tag_types};
use crate::{
    app::{ProblemLine, use_ui},
    backend::{Keys, Problem},
    field::{Area, Field, Kind},
    markdown::Markdown,
    model::Section,
};

/// How long typing in the tag finder must pause before it looks.
const FIND_AFTER_MS: u64 = 250;
/// Tags shown under each type before searching.
const PER_TYPE: u32 = 12;

/// A tag on the form: known to the server, or new (made when posting).
#[derive(Debug, Clone, PartialEq)]
struct Chosen {
    tag: Tag,
    new: bool,
}

impl Chosen {
    /// How the API takes it: `namespace:display`.
    fn spec(&self) -> String {
        format!("{}:{}", self.tag.namespace, self.tag.display)
    }
}

/// What still stops the form from posting, in words.
fn missing(title: &str, body: &str, types: &[TagNamespace], chosen: &[Chosen]) -> Vec<String> {
    let mut out = Vec::new();
    if title.trim().is_empty() {
        out.push("a title".to_owned());
    }
    if body.trim().is_empty() {
        out.push("the prompt's text".to_owned());
    }
    for t in types {
        let n = u32::try_from(chosen.iter().filter(|c| c.tag.namespace == t.name).count())
            .unwrap_or(u32::MAX);
        if !t.allows(n) {
            if n < t.min {
                out.push(format!(
                    "{} {} tag{}",
                    t.min - n,
                    t.display.to_lowercase(),
                    plural(t.min - n)
                ));
            } else {
                out.push(format!(
                    "fewer {} tags ({})",
                    t.display.to_lowercase(),
                    t.limit_label()
                ));
            }
        }
    }
    out
}

fn plural(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// A new tag typed as "type: name", or `None` when it has no known type.
fn typed_tag(text: &str, types: &[TagNamespace]) -> Option<Tag> {
    let (ty, name) = text.split_once(':')?;
    let (ty, name) = (ty.trim(), name.trim());
    let t = types
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(ty) || t.display.eq_ignore_ascii_case(ty))?;
    (!name.is_empty()).then(|| Tag {
        namespace: t.name.clone(),
        slug: name.to_lowercase(),
        display: name.to_owned(),
        origin: TagOrigin::User,
        status: TagStatus::Pending,
        color: t.color.clone(),
    })
}

#[component]
pub fn PostForm(editing: Option<Uuid>) -> Element {
    let (backend, mut model) = use_ui();
    let mut board = use_board();
    let epoch = use_board_epoch();
    use_tag_types();
    let original: Option<Prompt> =
        editing.and_then(|id| board.peek().items.iter().find(|p| p.id == id).cloned());
    let ready: Vec<(Uuid, String)> = model
        .read()
        .personas()
        .iter()
        .filter(|p| p.keys == Keys::Ready)
        .map(|p| (p.persona.id, p.persona.handle.clone()))
        .collect();
    let in_use = model.read().persona;

    let orig = original.clone();
    let mut title = use_signal(|| orig.as_ref().map(|p| p.title.clone()).unwrap_or_default());
    let orig = original.clone();
    let mut body = use_signal(|| orig.as_ref().map(|p| p.body.clone()).unwrap_or_default());
    let orig = original.clone();
    let mut maturity = use_signal(|| orig.as_ref().map_or(Maturity::DEFAULT_MAX, |p| p.maturity));
    let orig = original.clone();
    let mut state = use_signal(|| orig.as_ref().map_or(PromptState::Open, |p| p.state));
    let first = in_use.or_else(|| ready.first().map(|(id, _)| *id));
    let mut persona = use_signal(|| first);
    let orig = original.clone();
    let chosen = use_signal(|| {
        orig.map(|p| {
            p.tags
                .into_iter()
                .map(|tag| Chosen { tag, new: false })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
    });
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);

    if editing.is_none() && ready.is_empty() {
        return rsx! {
            h1 { "Post a prompt" }
            div { class: "nudge",
                p { "Prompts are posted as a persona, and none of yours is ready on this computer yet." }
                button { class: "primary", onclick: move |_| model.write().go(Section::Personas), "Make a persona" }
                button { class: "quiet", onclick: move |_| board.write().view = View::List, "Back to the board" }
            }
        };
    }
    if editing.is_some() && original.is_none() {
        return rsx! {
            p { class: "problem", "That prompt is gone: it was deleted, or isn't shown to you any more." }
            button { class: "secondary", onclick: move |_| board.write().view = View::List, "Back to the board" }
        };
    }

    let types = board.read().types.clone().unwrap_or_default();
    let lacking = missing(&title(), &body(), &types, &chosen.read());
    let can_post = lacking.is_empty() && !busy();

    let b = backend.clone();
    let submit = move |_| {
        if !missing(
            &title(),
            &body(),
            &board.peek().types.clone().unwrap_or_default(),
            &chosen.peek(),
        )
        .is_empty()
        {
            return;
        }
        busy.set(true);
        problem.set(None);
        let tags: Vec<String> = chosen.peek().iter().map(Chosen::spec).collect();
        let b = b.clone();
        spawn(async move {
            let result = match editing {
                Some(id) => {
                    let update = UpdatePrompt {
                        title: Some(title()),
                        body: Some(body()),
                        maturity: Some(maturity()),
                        tags: Some(tags),
                        state: Some(state()),
                    };
                    b.update_prompt(id, update).await
                }
                None => {
                    let Some(p) = persona() else {
                        busy.set(false);
                        return;
                    };
                    let create = CreatePrompt {
                        persona: p,
                        title: title(),
                        body: body(),
                        maturity: maturity(),
                        tags,
                        state: Some(state()),
                    };
                    b.create_prompt(create).await
                }
            };
            busy.set(false);
            match result {
                Ok(prompt) => {
                    posted(board, prompt, editing.is_some());
                    // The other list loads again when next shown.
                    epoch.changed(board);
                }
                // The form keeps everything typed, to fix and try again.
                Err(p) => problem.set(Some(p)),
            }
        });
    };

    let heading = if editing.is_some() {
        "Edit your prompt"
    } else {
        "Post a prompt"
    };
    let action = match (editing.is_some(), busy()) {
        (_, true) => "Saving…",
        (true, false) => "Save changes",
        (false, false) => "Post prompt",
    };
    let by = ready
        .iter()
        .find(|(id, _)| Some(*id) == persona())
        .map(|(_, h)| format!("@{h}"))
        .or_else(|| original.as_ref().map(|p| format!("@{}", p.author.handle)))
        .unwrap_or_default();
    rsx! {
        div { class: "post",
            section { class: "post-form",
                h1 { "{heading}" }
                Field {
                    label: "Title",
                    id: "post-title",
                    kind: Kind::Text,
                    initial: title.peek().clone(),
                    autofocus: editing.is_none(),
                    oninput: move |t| title.set(t),
                }
                if editing.is_none() {
                    div { class: "field",
                        label { r#for: "post-persona", "Post as" }
                        select {
                            id: "post-persona",
                            onchange: move |e| persona.set(e.value().parse().ok()),
                            for (id, handle) in ready.iter() {
                                option { key: "{id}", value: "{id}", selected: Some(*id) == persona(), "@{handle}" }
                            }
                        }
                    }
                }
                Area {
                    label: "Prompt",
                    id: "post-body",
                    initial: body.peek().clone(),
                    hint: "Markdown: *italic*, **bold**, lists and links.",
                    oninput: move |t| body.set(t),
                }
                div { class: "field-row",
                    div { class: "field",
                        label { r#for: "post-maturity", "Maturity rating" }
                        select {
                            id: "post-maturity",
                            onchange: move |e| if let Ok(m) = e.value().parse() { maturity.set(m) },
                            for m in Maturity::ALL.iter().copied() {
                                option { key: "{m}", value: "{m}", selected: m == maturity(), "{maturity_label(m)}" }
                            }
                        }
                    }
                    div { class: "field",
                        label { r#for: "post-status", "Status" }
                        select {
                            id: "post-status",
                            onchange: move |e| if let Ok(s) = e.value().parse() { state.set(s) },
                            for s in PromptState::ALL.iter().copied() {
                                option { key: "{s}", value: "{s}", selected: s == state(), "{state_label(s)}" }
                            }
                        }
                    }
                }
                ChosenTags { chosen }
                if let Some(p) = problem() {
                    ProblemLine { problem: p }
                }
                div { class: "actions",
                    button { class: "primary", disabled: !can_post, onclick: submit, "{action}" }
                    button {
                        class: "quiet",
                        onclick: move |_| board.write().view = View::List,
                        "Cancel"
                    }
                    if !lacking.is_empty() {
                        span { class: "hint", role: "status", "Needs {lacking.join(\", \")} before posting." }
                    }
                }
            }
            aside { class: "post-side",
                section { class: "preview", aria_label: "Preview",
                    h2 { "Preview" }
                    h3 { if title().trim().is_empty() { "Your title" } else { "{title}" } }
                    p { class: "reader-meta", "{by} · {maturity_label(maturity())} · {state_label(state())}" }
                    Markdown { source: body() }
                    div { class: "chips",
                        for c in chosen.read().iter() {
                            Chip { key: "{c.tag.key()}", tag: c.tag.clone() }
                        }
                    }
                }
                TagPanel { chosen, persona: persona() }
            }
        }
    }
}

/// After posting or saving: back to the board with it open.
fn posted(mut board: Signal<BoardState>, prompt: Prompt, edited: bool) {
    let mut b = board.write();
    let id = prompt.id;
    if edited {
        b.replace(prompt);
    } else {
        b.items.insert(0, prompt);
    }
    b.open = Some(id);
    b.view = View::List;
    b.said = Some(
        if edited {
            "Changes saved."
        } else {
            "Prompt posted."
        }
        .to_owned(),
    );
}

/// The tags on the form, and a dashed slot for each required type that
/// still needs one.
#[component]
fn ChosenTags(chosen: Signal<Vec<Chosen>>) -> Element {
    let board = use_board();
    let mut chosen = chosen;
    let policy = board.read().policy;
    let types = board.read().types.clone().unwrap_or_default();
    let list = chosen.read().clone();
    let remove = move |key: String| chosen.write().retain(|c| c.tag.key() != key);
    let slots: Vec<String> = types
        .iter()
        .filter_map(|t| {
            let n = u32::try_from(list.iter().filter(|c| c.tag.namespace == t.name).count())
                .unwrap_or(0);
            (n < t.min).then(|| format!("{}: pick {}", t.display, t.min - n))
        })
        .collect();
    rsx! {
        div { class: "field",
            span { class: "label", "Tags on this prompt" }
            div { class: "chips tag-box",
                for c in list.iter() {
                    Chip {
                        key: "{c.tag.key()}",
                        tag: c.tag.clone(),
                        on_remove: remove,
                        note: c.new.then(|| if policy == TagPolicy::Approval { "new, needs approval" } else { "new" }.to_owned()),
                    }
                }
                for s in slots {
                    span { key: "{s}", class: "slot", "{s}" }
                }
                if list.is_empty() && types.iter().all(|t| t.min == 0) {
                    span { class: "hint", "None yet. Add some from the panel." }
                }
            }
        }
    }
}

/// Finding and adding tags: a search over every type, the writer's
/// frequent tags, and each type with its count and limit.
#[component]
fn TagPanel(chosen: Signal<Vec<Chosen>>, persona: Option<Uuid>) -> Element {
    let (backend, _) = use_ui();
    let board = use_board();
    let mut chosen = chosen;
    let mut typed = use_signal(String::new);
    let mut found = use_signal(Vec::<Tag>::new);
    let mut note = use_signal(|| None::<String>);
    let mut round = use_signal(|| 0u32);
    let mut frequent = use_signal(Vec::<Tag>::new);
    let mut by_type = use_signal(Vec::<(String, Vec<Tag>)>::new);
    let types = board.read().types.clone().unwrap_or_default();
    let policy = board.read().policy;

    // Each type's most used tags, and the writer's frequent ones.
    let b = backend.clone();
    let names: Vec<String> = types.iter().map(|t| t.name.clone()).collect();
    use_effect(use_reactive!(|(names, persona)| {
        let b = b.clone();
        spawn(async move {
            if let Ok(tags) = b.frequent_tags(persona).await {
                frequent.set(tags);
            }
            let mut all = Vec::new();
            for ns in names {
                let q = TagPageQuery {
                    limit: Some(PER_TYPE),
                    ..TagPageQuery::default()
                };
                if let Ok(page) = b.tag_page(&ns, q).await {
                    all.push((ns, page.items.into_iter().map(|c| c.tag).collect()));
                }
            }
            by_type.set(all);
        });
    }));

    let count_of = move |ns: &str| -> u32 {
        u32::try_from(
            chosen
                .read()
                .iter()
                .filter(|c| c.tag.namespace == ns)
                .count(),
        )
        .unwrap_or(0)
    };
    // Adds `tag` unless it's on already or its type is full.
    let types_for_add = types.clone();
    let add = use_callback(move |(tag, new): (Tag, bool)| {
        if chosen.peek().iter().any(|c| c.tag.key() == tag.key()) {
            return;
        }
        if let Some(t) = types_for_add.iter().find(|t| t.name == tag.namespace) {
            let n = u32::try_from(
                chosen
                    .peek()
                    .iter()
                    .filter(|c| c.tag.namespace == t.name)
                    .count(),
            )
            .unwrap_or(0);
            if t.full(n) {
                note.set(Some(format!(
                    "{} is full ({}). Remove one to pick another.",
                    t.display,
                    t.limit_label()
                )));
                return;
            }
        }
        note.set(None);
        chosen.write().push(Chosen { tag, new });
    });

    let b = backend.clone();
    let on_find = use_callback(move |text: String| {
        typed.set(text.clone());
        let mine = round() + 1;
        round.set(mine);
        let prefix = text
            .split_once(':')
            .map_or(text.as_str(), |(_, n)| n)
            .trim()
            .to_owned();
        if prefix.is_empty() {
            found.set(Vec::new());
            return;
        }
        let b = b.clone();
        spawn(async move {
            b.sleep(FIND_AFTER_MS).await;
            if round() != mine {
                return;
            }
            let q = TagQuery {
                prefix: Some(prefix),
                limit: Some(12),
                ..TagQuery::default()
            };
            if let Ok(tags) = b.find_tags(q).await
                && round() == mine
            {
                found.set(tags);
            }
        });
    });
    let types_for_new = types.clone();
    let add_typed = use_callback(move |()| {
        let text = typed.peek().clone();
        let lower = text.trim().to_lowercase();
        // An existing tag by its exact name wins over making a new one.
        if let Some(t) = found.peek().iter().find(|t| {
            t.display.to_lowercase() == lower
                || t.key() == lower
                || format!("{}: {}", t.namespace, t.display.to_lowercase()) == lower
        }) {
            add.call((t.clone(), false));
            return;
        }
        match typed_tag(&text, &types_for_new) {
            None => note.set(Some(
                "For a new tag, type its type first, like theme: memory.".to_owned(),
            )),
            Some(_) if policy == TagPolicy::Curated => {
                note.set(Some(
                    "This server only takes its own tags. Pick one from the lists.".to_owned(),
                ));
            }
            Some(t)
                if types_for_new
                    .iter()
                    .any(|ty| ty.name == t.namespace && ty.locked) =>
            {
                note.set(Some(format!(
                    "{} only takes the server's own tags.",
                    t.namespace
                )));
            }
            Some(t) => add.call((t, true)),
        }
    });
    let list = chosen.read().clone();
    let is_on = |t: &Tag| list.iter().any(|c| c.tag.key() == t.key());
    rsx! {
        section { class: "tag-panel", aria_label: "Add tags",
            h2 { "Add tags" }
            div { class: "tag-find",
                input {
                    id: "tag-find",
                    r#type: "search",
                    aria_label: "Find a tag, or type a new one as type: name",
                    placeholder: "Find a tag, or type a new one",
                    autocomplete: "off",
                    "writingsuggestions": "false",
                    spellcheck: "false",
                    oninput: move |e| on_find.call(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter {
                            e.prevent_default();
                            add_typed.call(());
                        }
                    },
                }
                button { class: "secondary", onclick: move |_| add_typed.call(()), "Add" }
            }
            if let Some(n) = note() {
                p { class: "hint", role: "status", "{n}" }
            }
            if !found().is_empty() {
                div { class: "chips", role: "group", aria_label: "Matching tags",
                    for t in found() {
                        TagButton { key: "{t.key()}", tag: t.clone(), on: is_on(&t), on_add: move |t| add.call((t, false)) }
                    }
                }
            }
            if !frequent().is_empty() {
                h3 { "You often use" }
                div { class: "chips",
                    for t in frequent() {
                        TagButton { key: "{t.key()}", tag: t.clone(), on: is_on(&t), on_add: move |t| add.call((t, false)) }
                    }
                }
            }
            for t in types.iter() {
                {
                    let n = count_of(&t.name);
                    let status = if t.allows(n) && t.min > 0 {
                        "✓"
                    } else if t.full(n) {
                        "· full"
                    } else {
                        ""
                    };
                    // A type with no limits needs no count until something is picked.
                    let label = match (t.min, t.max, n) {
                        (0, None, 0) => String::new(),
                        (0, None, n) => format!("{n} picked"),
                        _ => format!("{n} of {}", limit_label(t.min, t.max)),
                    };
                    let tags = by_type.read().iter().find(|(ns, _)| *ns == t.name).map(|(_, v)| v.clone()).unwrap_or_default();
                    rsx! {
                        div { key: "{t.name}", class: if t.min > n { "tag-type needed" } else { "tag-type" },
                            div { class: "tag-type-head",
                                h3 { "{t.display}" }
                                span { class: "hint", "{label} {status}" }
                            }
                            div { class: "chips",
                                for tag in tags {
                                    TagButton { key: "{tag.key()}", tag: tag.clone(), on: is_on(&tag), short: true, on_add: move |t| add.call((t, false)) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A tag to add, or shown as on already.
#[component]
fn TagButton(
    tag: Tag,
    on: bool,
    /// Under its type's heading: shows the name alone, but is still named
    /// with its type for screen readers.
    #[props(default)]
    short: bool,
    on_add: EventHandler<Tag>,
) -> Element {
    let label = format!("{}: {}", tag.namespace, tag.display);
    let shown = if short {
        tag.display.clone()
    } else {
        label.clone()
    };
    rsx! {
        button {
            class: if on { "tag-option on" } else { "tag-option" },
            aria_label: if on { "{label} ✓" } else { "{label}" },
            aria_pressed: if on { "true" } else { "false" },
            disabled: on,
            onclick: move |_| on_add.call(tag.clone()),
            "{shown}"
            if on { " ✓" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ty(name: &str, min: u32, max: Option<u32>) -> TagNamespace {
        TagNamespace {
            name: name.into(),
            display: name[..1].to_uppercase() + &name[1..],
            locked: false,
            min,
            max,
            color: None,
        }
    }

    #[test]
    fn missing_says_what_stops_posting() {
        let types = [ty("length", 1, Some(1)), ty("theme", 0, Some(1))];
        let theme = |d: &str| Chosen {
            tag: typed_tag(&format!("theme: {d}"), &types).unwrap(),
            new: true,
        };
        assert_eq!(
            missing("", "", &types, &[]),
            ["a title", "the prompt's text", "1 length tag"]
        );
        assert_eq!(
            missing("T", "B", &types, &[theme("a"), theme("b")]),
            ["1 length tag", "fewer theme tags (up to 1)"]
        );
    }

    #[test]
    fn new_tags_need_a_known_type() {
        let types = [ty("theme", 0, None)];
        let t = typed_tag(" Theme : Memory ", &types).unwrap();
        assert_eq!(
            (t.namespace.as_str(), t.display.as_str()),
            ("theme", "Memory")
        );
        assert!(typed_tag("memory", &types).is_none());
        assert!(typed_tag("mood: calm", &types).is_none());
    }
}
