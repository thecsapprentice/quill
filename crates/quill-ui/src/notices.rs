//! The Notices section (mockup M2b): what administrators did to the
//! login's prompts, tags and comments. Opening it marks them read.

use dioxus::prelude::*;
use quill_proto::notices::AccountNotice;
use uuid::Uuid;

use crate::{
    app::{ProblemLine, use_ui},
    backend::Problem,
    board::MineState,
    model::Section,
};

/// Notices per page, as the server sends when none is asked for.
const PAGE: usize = 50;

#[component]
pub fn NoticesPage() -> Element {
    let (backend, mut model) = use_ui();
    let mut list = use_signal(|| None::<Vec<AccountNotice>>);
    let mut older = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut problem = use_signal(|| None::<Problem>);

    let b = backend.clone();
    let load = use_callback(move |before: Option<i64>| {
        busy.set(true);
        let b = b.clone();
        spawn(async move {
            let result = b.notices(before).await;
            busy.set(false);
            match result {
                Ok(page) => {
                    problem.set(None);
                    older.set(page.len() == PAGE);
                    let fresh = page.iter().any(|n| !n.read);
                    list.with_mut(|l| l.get_or_insert_with(Vec::new).extend(page));
                    // Seen now: the server and the nav count agree.
                    if fresh && b.mark_notices_read().await.is_ok() {
                        let id = model.peek().current;
                        if let Some(id) = id {
                            model.write().unread_loaded(id, 0);
                        }
                    }
                }
                Err(p) => problem.set(Some(p)),
            }
        });
    });
    use_hook(move || load.call(None));

    let shown = list();
    let last = shown.as_ref().and_then(|l| l.last()).map(|n| n.id);
    rsx! {
        h1 { "Notices" }
        p { class: "lede",
            "What this server's administrators have done to your prompts, tags and comments. Only you can see these. They never say who reported a prompt or which administrator acted."
        }
        if let Some(p) = problem() {
            ProblemLine { problem: p }
            div { class: "actions",
                button { class: "secondary", disabled: busy(), onclick: move |_| load.call(last), "Try again" }
            }
        }
        match shown {
            None if problem().is_none() => rsx! { p { class: "coming", "Loading notices…" } },
            None => rsx! {},
            Some(l) if l.is_empty() => rsx! { p { class: "coming", "Nothing yet." } },
            Some(l) => rsx! {
                ul { class: "notices",
                    for n in l {
                        Notice { key: "{n.id}", notice: n }
                    }
                }
                if older() {
                    div { class: "actions",
                        button {
                            class: "secondary",
                            disabled: busy(),
                            onclick: move |_| load.call(last),
                            if busy() { "Loading…" } else { "Older notices" }
                        }
                    }
                }
            },
        }
    }
}

#[component]
fn Notice(notice: AccountNotice) -> Element {
    let (_, mut model) = use_ui();
    let MineState(mut mine) = use_context::<MineState>();
    let (what, intro) = quill_board_ui::notice_sentence(&notice);
    let date = quill_board_ui::short_date(notice.at);
    // A notice's prompts open in My prompts, the one place a removed
    // prompt still shows.
    let mut open = move |id: Uuid| {
        {
            let mut b = mine.write();
            b.open = Some(id);
            b.persona = None;
            b.view = crate::board::View::List;
        }
        model.write().go(Section::MyPrompts);
    };
    rsx! {
        li { class: if notice.read { "notice-item" } else { "notice-item unread" },
            p { class: "hint",
                "{date}"
                if !notice.read { span { class: "new", " · New" } }
            }
            p { "{what}" }
            if notice.fields.announcement.is_some() {
                p { class: "hint",
                    "Only you can still see it there, marked as removed."
                    if let Some(handle) = &notice.fields.handle { " It was written as @{handle}." }
                }
            }
            if !notice.prompts.is_empty() {
                p { class: "hint", "{intro}" }
                ul { class: "notice-prompts",
                    for p in notice.prompts.iter() {
                        {
                            let id = p.id;
                            rsx! {
                                li { key: "{p.id}",
                                    button { class: "link", onclick: move |_| open(id), "{p.title}" }
                                    span { class: "hint", " as @{p.handle}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
