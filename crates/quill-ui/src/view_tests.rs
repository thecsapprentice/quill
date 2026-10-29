//! The views rendered to HTML with a fake backend: labels, roles, the
//! text-input rules from 3a, and no ids on screen outside advanced mode.

use std::rc::Rc;

use dioxus::prelude::*;

use crate::{
    app::{App, StartWith, Ui},
    backend::Keys,
    fake::{Fake, persona},
    model::{Model, Screen, Section},
};

/// Renders the app from `model` (or from the backend's settings).
fn render(fake: Fake, model: Option<Model>) -> String {
    let mut dom = VirtualDom::new(App);
    dom.provide_root_context(Ui(Rc::new(fake)));
    if let Some(m) = model {
        dom.provide_root_context(StartWith(m));
    }
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

/// The model once the fake's server is open.
fn opened(fake: &Fake) -> Model {
    let mut m = Model::load(fake);
    let id = m.current.unwrap();
    m.opening(id);
    m.opened(id, Ok(fake.standing.borrow().clone()));
    m
}

/// The text a writer sees: the HTML without its tags (and so without
/// attribute values such as option values).
fn visible(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Whether `text` holds something shaped like a UUID.
fn has_uuid(text: &str) -> bool {
    let b = text.as_bytes();
    let shape = [8, 4, 4, 4, 12];
    (0..b.len()).any(|start| {
        let mut i = start;
        for (n, len) in shape.iter().enumerate() {
            if i + len > b.len() || !b[i..i + len].iter().all(u8::is_ascii_hexdigit) {
                return false;
            }
            i += len;
            if n < 4 {
                if b.get(i) != Some(&b'-') {
                    return false;
                }
                i += 1;
            }
        }
        true
    })
}

/// The opening tag of the element with `id`.
fn tag_with_id<'a>(html: &'a str, id: &str) -> &'a str {
    let at = html
        .find(&format!("id=\"{id}\""))
        .unwrap_or_else(|| panic!("no #{id} in {html}"));
    let start = html[..at].rfind('<').unwrap();
    let end = at + html[at..].find('>').unwrap();
    &html[start..=end]
}

#[test]
fn a_fresh_install_asks_for_the_server_with_a_labelled_plain_field() {
    let html = render(Fake::empty(), None);
    assert!(html.contains("Welcome to Quill"), "{html}");
    assert!(html.contains(r#"for="server-address""#));
    let input = tag_with_id(&html, "server-address");
    for rule in [
        r#"autocomplete="off""#,
        r#"writingsuggestions="false""#,
        r#"spellcheck="false""#,
    ] {
        assert!(input.contains(rule), "{rule} missing: {input}");
    }
}

#[test]
fn the_first_persona_screen_offers_a_way_out() {
    let fake = Fake::signed_in(Vec::new());
    let mut m = opened(&fake);
    m.screen = Screen::FirstPersona;
    let html = render(fake, Some(m));
    assert!(html.contains("Make your first persona"));
    assert!(html.contains("Not now"));
    assert!(tag_with_id(&html, "persona-handle").contains(r#"spellcheck="false""#));
    // Display names are words: spellchecked.
    assert!(tag_with_id(&html, "persona-display-name").contains(r#"spellcheck="true""#));
}

#[test]
fn signing_up_keeps_the_password_managers_hint() {
    // A server just added: no login yet, so "new here" is first.
    let fake = Fake::signed_in(Vec::new());
    fake.servers.borrow_mut()[0].login = None;
    *fake.standing.borrow_mut() = crate::backend::Standing::SignedOut;
    let m = opened(&fake);
    assert_eq!(m.screen, Screen::Account);
    let html = render(fake, Some(m));
    assert!(html.contains("Make a login on writers.example"), "{html}");
    let password = tag_with_id(&html, "login-password");
    assert!(
        password.contains(r#"autocomplete="new-password""#),
        "{password}"
    );
    assert!(password.contains(r#"type="password""#));
}

#[test]
fn the_frame_names_its_parts_and_shows_no_ids() {
    let fake = Fake::signed_in(vec![
        persona("ink_owl", Keys::Ready),
        persona("ink_fox", Keys::Elsewhere),
    ]);
    let m = opened(&fake);
    assert_eq!(m.screen, Screen::Frame);
    let html = render(fake, Some(m));
    assert!(
        html.contains(r#"<nav class="nav" aria-label="Quill""#),
        "{html}"
    );
    assert!(html.contains(r#"aria-current="page""#));
    assert!(html.contains(r#"role="status""#));
    assert!(html.contains("Connected to writers.example"));
    assert!(html.contains("@ink_owl"));
    assert!(!has_uuid(&visible(&html)), "{}", visible(&html));
}

#[test]
fn personas_and_settings_show_ids_only_in_advanced_mode() {
    for advanced in [false, true] {
        for section in [Section::Personas, Section::Settings] {
            let fake = Fake::signed_in(vec![persona("ink_owl", Keys::Ready)]);
            let mut m = opened(&fake);
            m.settings.advanced = advanced;
            m.go(section);
            let html = render(fake, Some(m));
            let text = visible(&html);
            match section {
                Section::Personas => {
                    assert!(text.contains("@ink_owl"));
                    assert_eq!(has_uuid(&text), advanced, "{section:?} {text}");
                }
                _ => {
                    assert!(text.contains("Appearance"));
                    assert!(text.contains("Sign out of writers.example"));
                    // Details load later; until then nothing shows an id.
                    assert!(!has_uuid(&text), "{text}");
                }
            }
        }
    }
}

#[test]
fn keys_elsewhere_and_getting_ready_say_what_is_happening() {
    let fake = Fake::signed_in(vec![persona("ink_fox", Keys::Elsewhere)]);
    let m = opened(&fake);
    assert_eq!(m.screen, Screen::Elsewhere);
    let html = render(fake, Some(m));
    assert!(html.contains("@ink_fox"));
    assert!(html.contains("another of your devices"));

    let fake = Fake::signed_in(vec![persona("ink_fox", Keys::ToSetUp)]);
    let m = opened(&fake);
    let html = render(fake, Some(m));
    assert!(html.contains("Getting @ink_fox ready"), "{html}");
    assert_eq!(html.matches("<li").count(), 3);
}

#[test]
fn the_uuid_check_finds_uuids() {
    assert!(has_uuid("id 67e55044-10b1-426f-9247-bb680e5fe0c8 here"));
    assert!(!has_uuid("ink_fox 2026-10-26"));
}

fn rules(must_accept: bool) -> crate::backend::ServerRules {
    crate::backend::ServerRules {
        markdown: "# Be kind\n\nNo <b>spam</b>.".into(),
        version: 2,
        must_accept,
    }
}

#[test]
fn changed_rules_show_a_banner_and_an_accept_button() {
    let fake = Fake::signed_in(vec![persona("ink_owl", Keys::Ready)]);
    let mut m = opened(&fake);
    let id = m.current.unwrap();
    m.rules_loaded(id, Ok(Some(rules(true))));
    let html = render(Fake::signed_in(vec![]), Some(m.clone()));
    assert!(html.contains("The server rules have changed"), "{html}");

    m.go(Section::Rules);
    let html = render(fake, Some(m));
    assert!(html.contains("<h3>Be kind</h3>"), "{html}");
    assert!(html.contains("I accept the rules"));
    // The banner isn't repeated on the rules page itself.
    assert!(!html.contains("The server rules have changed"));
    // The source's HTML stays text.
    assert!(!html.contains("<b>spam"));
}

#[test]
fn accepted_rules_are_only_there_to_read() {
    let fake = Fake::signed_in(vec![persona("ink_owl", Keys::Ready)]);
    let mut m = opened(&fake);
    let id = m.current.unwrap();
    m.rules_loaded(id, Ok(Some(rules(false))));
    m.go(Section::Settings);
    let html = render(Fake::signed_in(vec![]), Some(m.clone()));
    assert!(html.contains("Server rules"), "{html}");
    m.go(Section::Rules);
    let html = render(fake, Some(m));
    assert!(!html.contains("I accept the rules"));
    assert!(!html.contains("The server rules have changed"));
}

#[component]
fn SignUpBox(required: bool) -> Element {
    rsx! {
        crate::server_rules::SignUpRules {
            rules: rules(false),
            required,
            accepted: false,
            on_accept: |_| {},
        }
    }
}

#[test]
fn sign_up_asks_to_accept_only_when_the_server_requires_it() {
    let render_box = |required| {
        let mut dom = VirtualDom::new_with_props(SignUpBox, SignUpBoxProps { required });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    };
    let required = render_box(true);
    assert!(required.contains("Read the server rules"), "{required}");
    assert!(required.contains(r#"id="accept-rules""#));
    let optional = render_box(false);
    assert!(!optional.contains(r#"id="accept-rules""#));
    assert!(optional.contains("you agree to follow the server rules"));
}

/// Opens the fake's server from a component that goes away at once, as
/// the add-server screen does when Cancel reopens the previous server.
#[component]
fn OpenThenLeave(status: Rc<std::cell::Cell<Option<crate::model::Status>>>) -> Element {
    let ui = use_context::<Ui>();
    let model = use_context_provider(|| Signal::new(Model::load(&*ui.0)));
    let mut showing = use_signal(|| true);
    let seen = status.clone();
    use_effect(move || seen.set(Some(model.read().status)));
    rsx! {
        if showing() {
            Leaving { on_left: move |_| showing.set(false) }
        }
    }
}

#[component]
fn Leaving(on_left: EventHandler<()>) -> Element {
    let (backend, model) = crate::app::use_ui();
    use_hook(move || {
        let id = model.peek().current.unwrap();
        crate::app::open_server(backend, model, id);
        on_left.call(());
    });
    rsx! {}
}

#[tokio::test]
async fn opening_a_server_finishes_after_its_screen_has_gone() {
    let status = Rc::new(std::cell::Cell::new(None));
    let mut dom = VirtualDom::new_with_props(
        OpenThenLeave,
        OpenThenLeaveProps {
            status: status.clone(),
        },
    );
    dom.provide_root_context(Ui(Rc::new(Fake::signed_in(vec![persona(
        "ink_owl",
        Keys::Ready,
    )]))));
    dom.rebuild_in_place();
    for _ in 0..20 {
        if status.get() == Some(crate::model::Status::Connected) {
            break;
        }
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work()).await;
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
    }
    assert_eq!(status.get(), Some(crate::model::Status::Connected));
}
