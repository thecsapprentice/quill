use quill_proto::{
    api::Persona,
    board::{
        AccountSettings, AdminAction, AdminLogEntry, AdminTag, Maturity, Prompt, PromptAuthor,
        PromptState, Report, ReportReason, Tag, TagNamespace, TagOrigin, TagPolicy,
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
            namespaces: vec![
                TagNamespace {
                    name: "genre".into(),
                    display: "Genre".into(),
                    locked: false,
                    required: true,
                },
                TagNamespace {
                    name: "format".into(),
                    display: "Format".into(),
                    locked: true,
                    required: true,
                },
                TagNamespace {
                    name: "length".into(),
                    display: "Length".into(),
                    locked: true,
                    required: false,
                },
            ],
        },
    );
    assert!(html.contains("Required: at least one Genre (genre:…) and Format (format:…) tag."));
    assert!(html.contains("except in format and length, which use"));
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
            namespaces: Vec::new(),
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

fn genre() -> TagNamespace {
    TagNamespace {
        name: "genre".into(),
        display: "Genre".into(),
        locked: false,
        required: true,
    }
}

fn admin_tag(slug: &str, uses: u64, alias_of: Option<&str>) -> AdminTag {
    AdminTag {
        tag: Tag {
            namespace: "genre".into(),
            slug: slug.into(),
            display: "<Horror>".into(),
            origin: TagOrigin::User,
        },
        uses,
        alias_of: alias_of.map(str::to_owned),
    }
}

#[test]
fn server_settings_pages() {
    let c = Chrome {
        current: Nav::ServerSettings,
        ..chrome(true, true)
    };
    let types = admin_tag_types_page(
        &c,
        AdminTagTypesView {
            namespaces: vec![genre()],
            new: NewTagType {
                name: "setting".into(),
                ..NewTagType::default()
            },
            error: Some("a tag type named \"setting\" already exists".into()),
        },
    );
    assert!(types.contains(r#"action="/admin/settings/tag-types/genre""#));
    assert!(types.contains(r#"name="required" value="yes" checked=true"#));
    assert!(types.contains(r#"value="setting""#));
    assert!(types.contains("role=\"alert\""));
    assert!(types.contains(r#"href="/admin/settings" aria-current="page""#));
    assert_eq!(types.matches(r#"name="csrf""#).count(), 3);

    let tags = admin_tags_page(
        &c,
        AdminTagsView {
            namespaces: vec![genre()],
            tags: vec![
                admin_tag("horror", 12, None),
                admin_tag("scary", 0, Some("genre:horror")),
            ],
            namespace: Some("genre".into()),
            search: "hor".into(),
            truncated: true,
            new_namespace: "genre".into(),
            new_name: String::new(),
            error: None,
        },
    );
    assert!(tags.contains("&#60;Horror&#62;"));
    assert!(tags.contains("12 prompts"));
    assert!(tags.contains("Other name for genre:horror"));
    assert!(tags.contains(r#"href="/admin/settings/tags/genre/horror/delete""#));
    assert!(tags.contains("Showing the first 2 tags"));

    let page = admin_tag_page(
        &c,
        AdminTagView {
            tag: admin_tag("horror", 1, None),
            namespace_display: "Genre".into(),
            rename: "Horror".into(),
            alias_target: String::new(),
            tag_options: Vec::new(),
            error: None,
        },
    );
    assert!(page.contains(r#"action="/admin/settings/tags/genre/horror/merge""#));
    assert!(page.contains("1 prompt"));
    let alias = admin_tag_page(
        &c,
        AdminTagView {
            tag: admin_tag("scary", 0, Some("genre:horror")),
            namespace_display: "Genre".into(),
            rename: "Scary".into(),
            alias_target: String::new(),
            tag_options: Vec::new(),
            error: None,
        },
    );
    assert!(!alias.contains("/merge"));

    let delete = admin_tag_delete_page(
        &c,
        AdminTagDeleteView {
            tag: admin_tag("horror", 3, None),
            namespace: genre(),
        },
    );
    assert!(delete.contains("It&#39;s on 3 prompts."));
    assert!(delete.contains("Genre is a required type."));
    assert!(delete.contains("Writers can still create this tag again"));
    assert!(delete.contains(r#"name="confirm""#));
}

#[test]
fn admin_log_shows_tag_and_report_entries() {
    let entry = |id, action, subject: Option<&str>, detail: Option<&str>| AdminLogEntry {
        id,
        at: datetime!(2026-10-22 12:00 UTC),
        action,
        report_id: None,
        prompt_id: subject.is_none().then(Uuid::nil),
        prompt_hash: subject.is_none().then(|| "00ff00ff00ff00ff".into()),
        reason: subject.is_none().then_some(ReportReason::Spam),
        subject: subject.map(str::to_owned),
        detail: detail.map(str::to_owned),
        admin: None,
    };
    let html = admin_log_page(
        &chrome(true, true),
        AdminLogView {
            entries: vec![
                entry(
                    2,
                    AdminAction::DeleteTag,
                    Some("genre:horror"),
                    Some("removed from 3 prompt(s)"),
                ),
                entry(1, AdminAction::ReportReceived, None, None),
            ],
            older: None,
        },
    );
    assert!(html.contains("Tag deleted"));
    assert!(html.contains("<code>genre:horror</code>"));
    assert!(html.contains("removed from 3 prompt(s)"));
    assert!(html.contains("Report received"));
    assert!(html.contains("<code>00ff00ff00ff</code>"));
    assert!(html.contains("View prompt"));
}
