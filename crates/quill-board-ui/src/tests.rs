use quill_proto::{
    api::Persona,
    board::{
        AccountSettings, Maturity, Prompt, PromptAuthor, PromptState, Report, ReportReason, Tag,
        TagOrigin, TagPolicy,
    },
};
use time::macros::datetime;
use uuid::Uuid;

use super::*;

fn chrome(signed_in: bool, admin: bool) -> Chrome {
    Chrome {
        site_name: "alpha.test".into(),
        viewer: signed_in.then(|| Viewer {
            csrf: "csrf-token-123".into(),
            is_admin: admin,
            maturity_max: Maturity::Teen,
        }),
        source_url: "https://github.com/thecsapprentice/quill".into(),
        version: "0.1.0".into(),
        notice: None,
        current: Nav::Board,
    }
}

fn prompt(title: &str, maturity: Maturity) -> Prompt {
    Prompt {
        id: Uuid::nil(),
        author: PromptAuthor {
            id: Uuid::nil(),
            handle: "ink_fox".into(),
            display_name: Some("Ink <Fox>".into()),
        },
        title: title.into(),
        body: "A *keeper*".into(),
        body_html: "<p>A <em>keeper</em></p>".into(),
        maturity,
        state: PromptState::Busy,
        tags: vec![Tag {
            namespace: "genre".into(),
            slug: "horror".into(),
            display: "Horror".into(),
            origin: TagOrigin::Server,
        }],
        accepting_sessions: false,
        removed: false,
        created_at: datetime!(2026-10-22 12:00 UTC),
        updated_at: datetime!(2026-10-22 12:00 UTC),
    }
}

fn board(prompts: Vec<Prompt>, filters: BoardFilters) -> BoardView {
    BoardView {
        filters,
        prompts,
        next_cursor: Some("abc+/=".into()),
        cap: Maturity::Teen,
        namespaces: Vec::new(),
        tag_options: Vec::new(),
        error: None,
    }
}

#[test]
fn board_escapes_text_and_keeps_sanitized_html() {
    let html = board_page(
        &chrome(false, false),
        board(
            vec![prompt("<script>alert(1)</script>", Maturity::General)],
            BoardFilters::default(),
        ),
    );
    assert!(html.starts_with("<!DOCTYPE html><html lang=\"en\">"));
    assert!(html.ends_with("</html>"));
    assert!(!html.contains("<script"));
    assert!(html.contains("&#60;script&#62;"));
    assert!(html.contains("Ink &#60;Fox&#62; (@ink_fox)"));
    // The server-sanitized body is inserted as HTML.
    assert!(html.contains("<p>A <em>keeper</em></p>"));
    // Tag links filter the board; the cursor is URL-encoded.
    assert!(html.contains(r#"href="/?include=genre%3Ahorror""#));
    assert!(html.contains("cursor=abc%2B%2F%3D"));
    assert!(html.contains("Busy"));
    assert!(html.contains(r#"href="https://github.com/thecsapprentice/quill""#));
    // Anonymous visitors get no forms that need a session.
    assert!(!html.contains("csrf-token-123"));
    assert!(html.contains(r#"href="/login""#));
}

#[test]
fn filter_urls_round_trip_and_open_the_panel() {
    let f = BoardFilters {
        include: "genre:horror,theme:sea".into(),
        q: "light house".into(),
        maturity: Some(Maturity::General),
        ..BoardFilters::default()
    };
    assert_eq!(
        f.url(None),
        "/?include=genre%3Ahorror%2Ctheme%3Asea&q=light+house&maturity=general"
    );
    assert_eq!(BoardFilters::default().url(None), "/");
    let html = board_page(&chrome(true, false), board(Vec::new(), f));
    assert!(html.contains("<details class=\"filters\" open"));
    assert!(html.contains("No prompts match these filters."));
}

#[test]
fn signed_in_pages_carry_the_csrf_token() {
    let c = chrome(true, true);
    let html = board_page(&c, board(Vec::new(), BoardFilters::default()));
    assert!(html.contains(r#"name="csrf" value="csrf-token-123""#)); // sign-out form
    assert!(html.contains(r#"href="/admin/reports""#));

    let html = prompt_page(
        &c,
        PromptView {
            prompt: prompt("Mine", Maturity::Teen),
            is_author: true,
            held_back: None,
        },
    );
    assert!(html.contains("/p/00000000-0000-0000-0000-000000000000/edit"));
    assert!(html.contains("/p/00000000-0000-0000-0000-000000000000/delete"));
    assert!(html.matches(r#"value="csrf-token-123""#).count() >= 3);
    assert!(!html.contains("Report this prompt"));
}

#[test]
fn prompts_above_the_cap_are_held_back() {
    let html = prompt_page(
        &chrome(false, false),
        PromptView {
            prompt: prompt("Night", Maturity::Explicit),
            is_author: false,
            held_back: Some(Maturity::Teen),
        },
    );
    assert!(html.contains("is rated Explicit, above your setting (Teen)"));
    assert!(html.contains("?show=1"));
    assert!(!html.contains("<em>keeper</em>"));
}

#[test]
fn forms_render_with_values_and_errors() {
    let persona = Persona {
        id: Uuid::nil(),
        handle: "ink_fox".into(),
        display_name: None,
        summary: None,
        created_at: datetime!(2026-10-22 12:00 UTC),
    };
    let html = prompt_form_page(
        &chrome(true, false),
        PromptFormView {
            editing: None,
            personas: vec![persona.clone()],
            persona: Some(persona.id),
            title: "T\"itle".into(),
            body: "</textarea><script>".into(),
            maturity: Maturity::Mature,
            tags: "genre:horror".into(),
            state: PromptState::Open,
            error: Some("Tag \"x\" is invalid".into()),
            tag_options: Vec::new(),
            tag_policy: TagPolicy::Both,
        },
    );
    assert!(html.contains(r#"value="T&#34;itle""#));
    assert!(html.contains("&#60;/textarea&#62;&#60;script&#62;"));
    assert!(html.contains(r#"<option value="mature" selected"#));
    assert!(html.contains("role=\"alert\""));

    let empty = prompt_form_page(
        &chrome(true, false),
        PromptFormView {
            editing: None,
            personas: Vec::new(),
            persona: None,
            title: String::new(),
            body: String::new(),
            maturity: Maturity::General,
            tags: String::new(),
            state: PromptState::Open,
            error: None,
            tag_options: Vec::new(),
            tag_policy: TagPolicy::Both,
        },
    );
    assert!(empty.contains("Create a persona"));
}

#[test]
fn other_pages_render() {
    let c = chrome(true, true);
    let report = report_page(
        &c,
        ReportView {
            prompt: prompt("P", Maturity::General),
            reason: Some(ReportReason::Spam),
            details: String::new(),
            error: None,
        },
    );
    assert!(report.contains(r#"value="spam" required=true checked=true"#));
    assert!(!report.contains(r#"value="other" required=true checked"#));
    assert!(report.contains("Wrong maturity rating"));

    let settings = settings_page(
        &c,
        SettingsView {
            settings: AccountSettings {
                maturity_max: Maturity::Mature,
                is_admin: true,
            },
        },
    );
    assert!(settings.contains(r#"<option value="mature" selected"#));

    let admin = admin_reports_page(
        &c,
        AdminReportsView {
            reports: vec![Report {
                id: Uuid::nil(),
                prompt: Some(prompt("Spam", Maturity::General)),
                reason: ReportReason::Spam,
                details: Some("<b>bad</b>".into()),
                prompt_hash: "ab".repeat(32),
                created_at: datetime!(2026-10-22 12:00 UTC),
                resolved_at: None,
                resolution: None,
            }],
            all: false,
        },
    );
    assert!(admin.contains("&#60;b&#62;bad&#60;/b&#62;"));
    assert!(admin.contains(r#"value="discard""#));
    assert!(admin.contains(r#"value="delete""#));

    let auth = auth_page(
        &chrome(false, false),
        AuthView {
            mode: AuthMode::Register,
            username: "x".into(),
            error: None,
            registration_open: true,
        },
    );
    assert!(auth.contains(r#"autocomplete="new-password""#));
    assert!(auth.contains("can&#39;t be recovered"));
}

#[test]
fn short_dates() {
    assert_eq!(short_date(datetime!(2026-01-05 00:00 UTC)), "Jan 5, 2026");
    assert_eq!(short_date(datetime!(2026-12-31 23:59 UTC)), "Dec 31, 2026");
}
