use quill_proto::{
    api::Persona,
    board::{
        AccountSettings, AdminAction, AdminLogDetail, AdminLogEntry, AdminLogPrompt,
        AdminLogTarget, AdminTag, LogActor, Maturity, Prompt, PromptAuthor, PromptState, Report,
        ReportKind, ReportReason, Tag, TagNamespace, TagOrigin, TagPolicy, TagStatus,
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
            pending_tags: if admin { 3 } else { 0 },
            unread_notices: 2,
            must_accept_rules: false,
        }),
        source_url: "https://github.com/thecsapprentice/quill".into(),
        version: "0.1.0".into(),
        notice: None,
        current: Nav::Board,
        has_rules: false,
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
            status: TagStatus::Visible,
            color: None,
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
        sticky_announcements: Vec::new(),
        announcement: None,
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
        actor_uri: "https://localhost/users/ink_fox".into(),
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
            chips: vec![
                TagChip {
                    value: "genre:horror".into(),
                    tag: Tag {
                        namespace: "genre".into(),
                        slug: "horror".into(),
                        display: "Horror".into(),
                        origin: TagOrigin::Server,
                        status: TagStatus::Visible,
                        color: None,
                    },
                    kind: ChipKind::Existing,
                },
                TagChip {
                    value: "theme:Sea <b>".into(),
                    tag: Tag {
                        namespace: "theme".into(),
                        slug: "sea-b".into(),
                        display: "Sea <b>".into(),
                        origin: TagOrigin::User,
                        status: TagStatus::Visible,
                        color: None,
                    },
                    kind: ChipKind::NewNeedsApproval,
                },
            ],
            add_text: "nope".into(),
            tag_errors: vec!["Tag \"nope\" must be written as namespace:name.".into()],
            state: PromptState::Open,
            error: Some("Tag \"x\" is invalid".into()),
            tag_options: vec![
                Tag {
                    namespace: "format".into(),
                    slug: "prose".into(),
                    display: "Prose".into(),
                    origin: TagOrigin::Server,
                    status: TagStatus::Visible,
                    color: None,
                },
                Tag {
                    namespace: "genre".into(),
                    slug: "horror".into(),
                    display: "Horror".into(),
                    origin: TagOrigin::Server,
                    status: TagStatus::Visible,
                    color: None,
                },
            ],
            tag_policy: TagPolicy::Open,
            namespaces: vec![
                TagNamespace {
                    name: "genre".into(),
                    display: "Genre".into(),
                    locked: false,
                    min: 1,
                    max: None,
                    color: None,
                },
                TagNamespace {
                    name: "format".into(),
                    display: "Format".into(),
                    locked: true,
                    min: 1,
                    max: None,
                    color: None,
                },
                TagNamespace {
                    name: "length".into(),
                    display: "Length".into(),
                    locked: true,
                    min: 0,
                    max: None,
                    color: None,
                },
            ],
        },
    );
    // Chips: a ticked checkbox each, escaped, with their kind.
    assert!(html.contains(r#"name="tag" value="genre:horror" checked=true"#));
    assert!(html.contains(r#"value="theme:Sea &#60;b&#62;""#));
    assert!(html.contains("(new, needs approval)"));
    assert!(html.contains("chip tcn-genre tc-genre__horror chip-choice"));
    // Genre is chosen; Format is required and missing, so it gets a slot.
    assert!(html.contains("Format: required"));
    assert!(!html.contains("Genre: required"));
    // Typed text that couldn't be added is kept, with the reason.
    assert!(html.contains(r#"name="add" value="nope""#));
    assert!(html.contains("must be written as namespace:name"));
    // Add comes before Post, so Enter adds; both reload to the picker.
    let add = html.find(r#"value="add""#).unwrap();
    assert!(add < html.find(r#"value="save""#).unwrap());
    assert!(html.contains(r##"formaction="/new#tags""##));
    // Browse offers what isn't chosen yet, required types first.
    assert!(html.contains(r#"name="pick" value="format:prose""#));
    assert!(!html.contains(r#"name="pick" value="genre:horror""#));
    assert!(html.contains("except in format and length, which use"));
    assert!(html.contains(r#"href="/static/tag-colors.css""#));
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
            chips: Vec::new(),
            add_text: String::new(),
            tag_errors: Vec::new(),
            state: PromptState::Open,
            error: None,
            tag_options: Vec::new(),
            tag_policy: TagPolicy::Open,
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
                unread_notices: 0,
                staff_name: None,
            },
            staff_name: "Nat <admin>".into(),
            error: Some("a staff name must be".into()),
        },
    );
    assert!(settings.contains(r#"<option value="mature" selected"#));
    // Administrators get the staff name field, kept on error.
    assert!(settings.contains(r#"name="staff_name""#));
    assert!(settings.contains("Nat &#60;admin&#62;"));
    assert!(settings.contains("a staff name must be"));

    let admin = admin_reports_page(
        &c,
        AdminReportsView {
            reports: vec![Report {
                id: Uuid::nil(),
                kind: ReportKind::Prompt,
                prompt: Some(prompt("Spam", Maturity::General)),
                comment: None,
                reason: ReportReason::Spam,
                details: Some("<b>bad</b>".into()),
                prompt_hash: "ab".repeat(32),
                created_at: datetime!(2026-10-22 12:00 UTC),
                resolved_at: None,
                resolution: None,
                script: None,
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
            accept_rules: None,
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
        min: 1,
        max: None,
        color: None,
    }
}

fn admin_tag(slug: &str, uses: u64, alias_of: Option<&str>) -> AdminTag {
    AdminTag {
        tag: Tag {
            namespace: "genre".into(),
            slug: slug.into(),
            display: "<Horror>".into(),
            origin: TagOrigin::User,
            status: TagStatus::Visible,
            color: None,
        },
        uses,
        alias_of: alias_of.map(str::to_owned),
        own_color: None,
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
            new_tags: TagPolicy::Approval,
            namespaces: vec![genre()],
            outside: vec![("genre".into(), 2)],
            new: NewTagType {
                name: "setting".into(),
                ..NewTagType::default()
            },
            pending: None,
            error: Some("a tag type named \"setting\" already exists".into()),
        },
    );
    assert!(types.contains(r#"action="/admin/settings/tag-types/genre""#));
    // Its limits have a form of their own, with quick choices.
    assert!(types.contains(r#"action="/admin/settings/tag-types/genre/limits""#));
    assert!(types.contains("Every prompt needs at least 1."));
    assert!(types.contains(r#"value="exactly-one""#));
    assert!(types.contains(r#"id="ns-genre-min" name="min" min="0" max="15" value="1""#));
    assert!(types.contains("2 prompts don&#39;t meet these limits yet."));
    assert!(types.contains(r#"href="/admin/settings/tag-types/genre/outside""#));
    assert!(types.contains(r#"value="setting""#));
    assert!(types.contains("role=\"alert\""));
    assert!(types.contains(r#"href="/admin/settings" aria-current="page""#));
    // The header counts waiting tag requests for administrators.
    assert!(types.contains(r#"<span class="count" title="Tag requests waiting">3</span>"#));
    // The new-tags switch, set to approval; plus the type's two forms and
    // the add form.
    assert!(types.contains(r#"value="approval" checked=true"#));
    assert_eq!(types.matches(r#"name="csrf""#).count(), 5);

    // New limits that leave prompts outside wait for a confirmation.
    let confirm = admin_tag_types_page(
        &c,
        AdminTagTypesView {
            new_tags: TagPolicy::Approval,
            namespaces: vec![genre()],
            outside: Vec::new(),
            new: NewTagType::default(),
            pending: Some(PendingLimits {
                namespace: "genre".into(),
                min: 1,
                max: Some(1),
                outside: 41,
            }),
            error: None,
        },
    );
    assert!(confirm.contains("41 prompts on the board wouldn&#39;t meet these limits."));
    assert!(confirm.contains(r#"name="confirm" value="yes""#));
    assert!(confirm.contains(r#"name="max" value="1""#));

    let outside = admin_outside_limits_page(
        &c,
        AdminOutsideLimitsView {
            namespace: TagNamespace {
                max: Some(1),
                ..genre()
            },
            total: 3,
            prompts: vec![(Uuid::nil(), "Too <many>".into())],
        },
    );
    assert!(outside.contains("Genre: exactly 1 per prompt"));
    assert!(outside.contains("Too &#60;many&#62;"));
    assert!(outside.contains("Showing the newest 1."));

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
    assert!(delete.contains("Every prompt needs at least 1 Genre tag."));
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
        comment_id: None,
        prompt_hash: subject.is_none().then(|| "00ff00ff00ff00ff".into()),
        reason: subject.is_none().then_some(ReportReason::Spam),
        subject: subject.map(str::to_owned),
        detail: detail.map(str::to_owned),
        admin: None,
        actor: if subject.is_some() {
            LogActor::Admin
        } else {
            LogActor::Reporter
        },
        admin_login: subject.is_some().then(|| "boss & co".to_owned()),
        script_name: None,
        // The server sends a preview of five and the full count.
        targets: if subject.is_some() {
            (1..=5).map(|i| format!("writer{i}")).collect()
        } else {
            vec!["alice".into()]
        },
        target_count: if subject.is_some() { 1207 } else { 1 },
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
            older: Some(admin_log_url(Some("boss & co"), None, Some(1))),
            admin: Some("boss & co".into()),
            account: None,
        },
    );
    // Who acted, as a filter link; reports don't name their writer.
    assert!(html.contains(r#"href="/admin/log?admin=boss+%26+co""#));
    assert!(html.contains(">boss &#38; co</a>"));
    assert!(html.contains("<em>A report</em>"));
    // Which accounts, five at most, then a count.
    assert!(html.contains(r#"href="/admin/log?account=writer1""#));
    assert!(!html.contains("account=writer6"));
    assert!(html.contains(r#"<a class="more" href="/admin/log/2">and 1202 more</a>"#));
    assert!(!html.contains("/admin/log/1\">and"));
    // Every entry links to its full page.
    assert!(html.contains(r#"href="/admin/log/1""#));
    assert!(html.contains("Oct 22, 2026 12:00 UTC"));
    // Filters are kept when paging.
    assert!(html.contains(r#"href="/admin/log?admin=boss+%26+co&#38;before=1""#));
    assert!(html.contains("Show everything"));
    assert!(html.contains("Tag deleted"));
    assert!(html.contains("<code>genre:horror</code>"));
    assert!(html.contains("removed from 3 prompt(s)"));
    assert!(html.contains("Report received"));
    assert!(html.contains("<code>00ff00ff00ff</code>"));
    assert!(html.contains("View prompt"));
}

#[test]
fn admin_log_entry_lists_accounts_and_their_prompts() {
    let entry = AdminLogEntry {
        id: 9,
        at: datetime!(2026-10-22 12:00 UTC),
        action: AdminAction::DeleteTag,
        report_id: None,
        prompt_id: None,
        comment_id: None,
        prompt_hash: None,
        reason: None,
        subject: Some("genre:horror".into()),
        detail: Some("removed from 3 prompt(s)".into()),
        admin: None,
        actor: LogActor::Admin,
        admin_login: Some("boss".into()),
        script_name: None,
        targets: vec!["alice".into(), "zed".into()],
        target_count: 250,
    };
    let prompt = |n: u128, title: &str, exists| AdminLogPrompt {
        id: Uuid::from_u128(n),
        title: title.into(),
        handle: "ink_fox".into(),
        exists,
    };
    let html = admin_log_entry_page(
        &chrome(true, true),
        AdminLogEntryView {
            detail: AdminLogDetail {
                entry,
                targets: vec![
                    AdminLogTarget {
                        login: "alice".into(),
                        account_deleted: false,
                        prompts: vec![prompt(1, "Lamps", true), prompt(2, "Tides <b>", false)],
                    },
                    AdminLogTarget {
                        login: "zed".into(),
                        account_deleted: true,
                        prompts: vec![],
                    },
                ],
                next: Some("zed".into()),
            },
            paged: true,
        },
    );
    assert!(html.contains("<h1>Tag deleted</h1>"));
    assert!(html.contains("250 accounts, alphabetically."));
    assert!(html.contains(r#"href="/admin/log?admin=boss""#));
    assert!(html.contains(r#"href="/admin/log?account=alice""#));
    assert!(html.contains(&format!(
        r#"href="/p/{}?show=1">Lamps</a>"#,
        Uuid::from_u128(1)
    )));
    // A removed prompt keeps its title, unlinked and escaped.
    assert!(html.contains("Tides &#60;b&#62; "));
    assert!(html.contains(" by @ink_fox"));
    assert!(!html.contains(&format!("/p/{}", Uuid::from_u128(2))));
    assert!(html.contains("Account deleted"));
    assert!(html.contains(r#"href="/admin/log/9?after=zed">Next accounts"#));
    assert!(html.contains(r#"href="/admin/log/9">First accounts"#));
}

#[test]
fn tags_only_some_viewers_see() {
    let mut p = prompt("Lamps", Maturity::General);
    let tag = |slug: &str, status| Tag {
        namespace: "theme".into(),
        slug: slug.into(),
        display: slug.into(),
        origin: TagOrigin::User,
        status,
        color: None,
    };
    p.tags = vec![
        tag("open", TagStatus::Visible),
        tag("waiting", TagStatus::Pending),
        tag("secret", TagStatus::Hidden),
    ];
    let html = prompt_page(
        &chrome(true, false),
        PromptView {
            prompt: p,
            is_author: true,
            held_back: None,
        },
    );
    assert!(html.contains(r#"href="/?include=theme%3Aopen""#));
    // Not offered as filters: they wouldn't work for anyone else.
    assert!(!html.contains("include=theme%3Awaiting"));
    assert!(!html.contains("include=theme%3Asecret"));
    assert!(html.contains("waiting <em>(awaiting approval)</em>"));
    assert!(html.contains("secret <em>(hidden)</em>"));
}

#[test]
fn tag_requests_and_status_controls() {
    let c = Chrome {
        current: Nav::ServerSettings,
        ..chrome(true, true)
    };
    let mut pending = admin_tag("selkies", 2, None);
    pending.tag.status = TagStatus::Pending;
    let requests = admin_tag_requests_page(
        &c,
        AdminTagRequestsView {
            requests: vec![pending.clone()],
            namespaces: vec![genre()],
            new_tags: TagPolicy::Open,
        },
    );
    assert!(requests.contains(r#"action="/admin/settings/tags/genre/selkies/status""#));
    assert!(requests.contains(r#"name="back" value="requests""#));
    assert!(requests.contains(r#"href="/admin/settings/tags/genre/selkies#merge""#));
    assert!(requests.contains(r#"href="/admin/settings/tags/genre/selkies/delete""#));
    assert!(requests.contains("so no new requests arrive"));

    let tag_view = |tag: AdminTag| AdminTagView {
        tag,
        namespace_display: "Genre".into(),
        rename: String::new(),
        alias_target: String::new(),
        tag_options: Vec::new(),
        error: None,
    };
    let page = admin_tag_page(&c, tag_view(pending.clone()));
    assert!(page.contains("Approve tag"));
    assert!(page.contains("Reject tag"));
    assert!(!page.contains(">Delete tag<"));
    let visible = admin_tag_page(&c, tag_view(admin_tag("horror", 1, None)));
    assert!(visible.contains("Hide tag"));
    let mut hidden = admin_tag("horror", 1, None);
    hidden.tag.status = TagStatus::Hidden;
    assert!(admin_tag_page(&c, tag_view(hidden)).contains("Show tag"));

    let reject = admin_tag_delete_page(
        &c,
        AdminTagDeleteView {
            tag: pending,
            namespace: genre(),
        },
    );
    assert!(reject.contains("Reject &#60;Horror&#62;?"));
    assert!(reject.contains(">Reject tag<"));
}

#[test]
fn notices_page_explains_actions() {
    use quill_proto::notices::{AccountNotice, NoticeFields, NoticeKind, NoticePrompt};
    let prompt = NoticePrompt {
        id: Uuid::nil(),
        title: "The <ledger>".into(),
        handle: "ink_fox".into(),
    };
    let notice = |id, kind, fields, read| AccountNotice {
        id,
        at: datetime!(2026-10-22 12:00 UTC),
        kind,
        fields,
        prompts: vec![prompt.clone()],
        read,
    };
    let html = notices_page(
        &chrome(true, false),
        NoticesView {
            notices: vec![
                notice(
                    3,
                    NoticeKind::PromptRemoved,
                    NoticeFields {
                        reason: Some(ReportReason::Spam),
                        ..NoticeFields::default()
                    },
                    false,
                ),
                notice(
                    2,
                    NoticeKind::TagMerged,
                    NoticeFields {
                        tag: Some("theme:selchies".into()),
                        tag_name: Some("Selchies".into()),
                        into: Some("theme:selkies".into()),
                        ..NoticeFields::default()
                    },
                    true,
                ),
            ],
            older: None,
        },
    );
    assert!(html.contains("removed your prompt from the board after a report (Spam)."));
    assert!(html.contains("merged the tag theme: Selchies into theme:selkies."));
    assert!(html.contains("The &#60;ledger&#62;"));
    assert!(html.contains(" as @ink_fox"));
    assert_eq!(html.matches(">New<").count(), 1);
    // The header counts unread notices.
    assert!(html.contains(r#"title="Unread notices">2</span>"#));
}

fn announcement(title: &str, edited: bool) -> quill_proto::announcements::Announcement {
    quill_proto::announcements::Announcement {
        id: Uuid::from_u128(7),
        title: title.into(),
        body: "Maintenance *tonight*".into(),
        body_html: "<p>Maintenance <em>tonight</em></p>".into(),
        comments_open: true,
        sticky: false,
        created_at: datetime!(2026-10-22 12:00 UTC),
        updated_at: if edited {
            datetime!(2026-10-24 09:00 UTC)
        } else {
            datetime!(2026-10-22 12:00 UTC)
        },
    }
}

#[test]
fn the_board_shows_the_newest_announcement_as_a_banner() {
    let mut view = board(Vec::new(), BoardFilters::default());
    let html = board_page(&chrome(false, false), view.clone());
    assert!(!html.contains("announcement-banner"));
    // Everyone gets the nav link, signed in or not.
    assert!(html.contains(r#"href="/announcements""#));

    view.announcement = Some(announcement("Downtime <Friday>", false));
    let html = board_page(&chrome(false, false), view);
    assert!(html.contains("announcement-banner"));
    assert!(html.contains("Downtime &#60;Friday&#62;"));
    assert!(html.contains(r#"href="/announcements/00000000-0000-0000-0000-000000000007""#));
    // Dismissing is a plain form: no account, no JavaScript.
    assert!(html.contains(r#"action="/announcements/dismiss""#));
    assert!(html.contains(r#"name="id" value="00000000-0000-0000-0000-000000000007""#));
}

#[test]
fn announcement_pages_render() {
    let html = announcements_page(
        &chrome(false, false),
        AnnouncementsView {
            announcements: vec![announcement("Welcome", false)],
            next_cursor: Some("next".into()),
        },
    );
    assert!(html.contains("From the administrators"));
    assert!(html.contains("<p>Maintenance <em>tonight</em></p>"));
    assert!(html.contains(r#"href="/announcements?cursor=next""#));
    assert!(!html.contains("Edited"));

    let empty = announcements_page(
        &chrome(false, false),
        AnnouncementsView {
            announcements: Vec::new(),
            next_cursor: None,
        },
    );
    assert!(empty.contains("No announcements yet."));

    // Readers see when it was edited; only administrators get an Edit link.
    let reader = announcement_page(
        &chrome(true, false),
        announcement_view(announcement("Welcome", true)),
    );
    assert!(reader.contains("Edited Oct 24, 2026"));
    assert!(!reader.contains("/admin/settings/announcements/"));
    let admin = announcement_page(
        &chrome(true, true),
        announcement_view(announcement("Welcome", true)),
    );
    assert!(
        admin.contains(
            r#"href="/admin/settings/announcements/00000000-0000-0000-0000-000000000007""#
        )
    );
}

#[test]
fn admin_announcement_pages_render() {
    let c = chrome(true, true);
    let html = admin_announcements_page(
        &c,
        AdminAnnouncementsView {
            announcements: vec![announcement("Welcome", false)],
            next_cursor: None,
            title: "Draft <title>".into(),
            body: "Draft body".into(),
            comments_open: false,
            sticky: true,
            error: Some("title is required".into()),
            comment_visibility: quill_proto::announcements::CommentVisibility::SignedIn,
        },
    );
    assert!(html.contains(r#"aria-current="page">Announcements"#));
    // The visibility setting shows its current value.
    assert!(html.contains(r#"action="/admin/settings/comment-visibility""#));
    assert!(html.contains(r#"value="signed-in" checked"#));
    assert!(!html.contains(r#"value="public" checked"#));
    assert!(html.contains("title is required"));
    assert!(html.contains(r#"value="Draft &#60;title&#62;""#));
    assert!(html.contains(r#"name="csrf" value="csrf-token-123""#));
    // Comments are open, so the button closes them.
    assert!(html.contains(r#"name="comments" value="off""#));
    assert!(html.contains("Close comments"));

    let mut closed = announcement("Welcome", false);
    closed.comments_open = false;
    let html = admin_announcement_page(
        &c,
        AdminAnnouncementView {
            title: closed.title.clone(),
            body: closed.body.clone(),
            announcement: closed,
            error: None,
        },
    );
    assert!(html.contains("Open comments"));
    assert!(html.contains(
        r#"action="/admin/settings/announcements/00000000-0000-0000-0000-000000000007/delete""#
    ));
}

fn announcement_view(announcement: quill_proto::announcements::Announcement) -> AnnouncementView {
    AnnouncementView {
        announcement,
        comments: Vec::new(),
        more_comments: None,
        personas: Vec::new(),
        draft: String::new(),
        speak_as: None,
        staff_name: None,
        error: None,
        comments_hidden: false,
    }
}

#[test]
fn hidden_comments_ask_the_visitor_to_sign_in() {
    let mut view = announcement_view(announcement("Welcome", false));
    view.comments_hidden = true;
    let html = announcement_page(&chrome(false, false), view);
    assert!(html.contains("comments are for signed-in writers"));
    assert!(html.contains(r#"href="/login""#));
    assert!(!html.contains("No comments yet."));
    assert!(!html.contains("to comment."));
    // The announcement itself still shows.
    assert!(html.contains("<p>Maintenance <em>tonight</em></p>"));
}

fn persona(id: u128, handle: &str) -> Persona {
    Persona {
        id: Uuid::from_u128(id),
        actor_uri: format!("https://alpha.test/users/{handle}"),
        handle: handle.into(),
        display_name: None,
        summary: None,
        created_at: datetime!(2026-10-22 12:00 UTC),
    }
}

fn comment(
    id: u128,
    author: u128,
    handle: &str,
    removed: bool,
) -> quill_proto::announcements::Comment {
    quill_proto::announcements::Comment {
        id: Uuid::from_u128(id),
        announcement: Uuid::from_u128(7),
        voice: quill_proto::announcements::CommentVoice::Persona,
        author: Some(PromptAuthor {
            id: Uuid::from_u128(author),
            handle: handle.into(),
            display_name: None,
        }),
        staff_name: None,
        body: "Thanks <b>all</b>".into(),
        body_html: "<p>Thanks &lt;b&gt;all&lt;/b&gt;</p>".into(),
        removed,
        created_at: datetime!(2026-10-23 08:00 UTC),
    }
}

#[test]
fn comments_render_under_an_announcement() {
    let mut view = announcement_view(announcement("Welcome", false));
    view.comments = vec![
        comment(100, 1, "ink_fox", false),
        comment(101, 2, "sea_wolf", true),
    ];
    view.more_comments = Some("/announcements/x?after=c".into());

    // Anonymous readers see comments and can report them, but not post.
    let html = announcement_page(&chrome(false, false), view.clone());
    assert!(html.contains(r#"id="c-00000000-0000-0000-0000-000000000064""#));
    assert!(html.contains("@ink_fox"));
    assert!(html.contains("<p>Thanks &lt;b&gt;all&lt;/b&gt;</p>"));
    assert!(html.contains(r#"href="/comments/00000000-0000-0000-0000-000000000064/report""#));
    assert!(html.contains("to comment."));
    assert!(html.contains("More comments"));
    assert!(!html.contains("Post comment"));

    // A writer gets the form, and a Delete button on their own comment
    // instead of Report. Their removed comment is marked.
    view.personas = vec![persona(2, "sea_wolf")];
    view.draft = "Kept <draft>".into();
    view.error = Some("body is required".into());
    let html = announcement_page(&chrome(true, false), view.clone());
    assert!(html.contains("Post comment"));
    assert!(html.contains("Kept &#60;draft&#62;"));
    assert!(html.contains("body is required"));
    assert!(html.contains(r#"action="/comments/00000000-0000-0000-0000-000000000065/delete""#));
    assert!(!html.contains(r#"href="/comments/00000000-0000-0000-0000-000000000065/report""#));
    assert!(html.contains("Removed by a server administrator"));
    assert!(!html.contains("/admin/comments/"));

    // Administrators can remove a comment that isn't removed yet.
    let html = announcement_page(&chrome(true, true), view.clone());
    assert!(
        html.contains(r#"action="/admin/comments/00000000-0000-0000-0000-000000000064/remove""#)
    );
    assert!(
        !html.contains(r#"action="/admin/comments/00000000-0000-0000-0000-000000000065/remove""#)
    );

    // Closed: the comments stay, the form goes.
    view.announcement.comments_open = false;
    let html = announcement_page(&chrome(true, false), view);
    assert!(html.contains("Comments are closed."));
    assert!(html.contains("@ink_fox"));
    assert!(!html.contains("Post comment"));
}

#[test]
fn comment_reports_and_notices_render() {
    let html = comment_report_page(
        &chrome(false, false),
        CommentReportView {
            comment: comment(100, 1, "ink_fox", false),
            reason: Some(ReportReason::Spam),
            details: String::new(),
            error: None,
        },
    );
    assert!(html.contains(r#"action="/comments/00000000-0000-0000-0000-000000000064/report""#));
    assert!(html.contains("Report a comment"));
    assert_eq!(
        html.matches(r#"name="reason""#).count(),
        ReportReason::ALL.len()
    );

    let report = Report {
        id: Uuid::nil(),
        kind: ReportKind::Comment,
        prompt: None,
        comment: Some(comment(100, 1, "ink_fox", false)),
        reason: ReportReason::Harassment,
        details: None,
        prompt_hash: "00ff".into(),
        created_at: datetime!(2026-10-22 12:00 UTC),
        resolved_at: None,
        resolution: None,
        script: None,
    };
    let html = admin_reports_page(
        &chrome(true, true),
        AdminReportsView {
            reports: vec![
                report.clone(),
                Report {
                    comment: None,
                    ..report
                },
            ],
            all: false,
        },
    );
    assert!(html.contains("Comment by @ink_fox"));
    assert!(html.contains("Remove comment"));
    assert!(html.contains("Comment deleted by its author"));

    let html = notices_page(
        &chrome(true, false),
        NoticesView {
            notices: vec![quill_proto::notices::AccountNotice {
                id: 1,
                at: datetime!(2026-10-22 12:00 UTC),
                kind: quill_proto::notices::NoticeKind::CommentRemoved,
                fields: quill_proto::notices::NoticeFields {
                    reason: Some(ReportReason::Spam),
                    announcement: Some(Uuid::from_u128(7)),
                    announcement_title: Some("Welcome".into()),
                    handle: Some("ink_fox".into()),
                    ..Default::default()
                },
                prompts: Vec::new(),
                read: false,
            }],
            older: None,
        },
    );
    assert!(html.contains("removed your comment on “Welcome” (Spam)."));
    assert!(html.contains(r#"href="/announcements/00000000-0000-0000-0000-000000000007""#));
    assert!(html.contains("(written as @ink_fox)"));
}

#[test]
fn sticky_banners_stack_and_cannot_be_dismissed() {
    let mut first = announcement("Rules updated", false);
    first.id = Uuid::from_u128(1);
    first.sticky = true;
    let mut second = announcement("Downtime Friday", false);
    second.id = Uuid::from_u128(2);
    second.sticky = true;
    let mut view = board(Vec::new(), BoardFilters::default());
    view.sticky_announcements = vec![second, first];
    view.announcement = Some(announcement("Welcome", false));
    let html = board_page(&chrome(false, false), view);

    assert_eq!(html.matches("announcement-banner sticky").count(), 2);
    // Newest sticky first, then the dismissible one; only it has the button.
    let downtime = html.find("Downtime Friday").unwrap();
    let rules = html.find("Rules updated").unwrap();
    let welcome = html.find("Welcome").unwrap();
    assert!(downtime < rules && rules < welcome);
    assert_eq!(
        html.matches(r#"action="/announcements/dismiss""#).count(),
        1
    );
    assert!(html.contains(r#"name="id" value="00000000-0000-0000-0000-000000000007""#));
}

#[test]
fn admins_make_announcements_sticky() {
    let c = chrome(true, true);
    let mut sticky = announcement("Rules updated", false);
    sticky.sticky = true;
    let html = admin_announcements_page(
        &c,
        AdminAnnouncementsView {
            announcements: vec![sticky.clone(), announcement("Welcome", false)],
            next_cursor: None,
            title: String::new(),
            body: String::new(),
            comments_open: true,
            sticky: false,
            error: None,
            comment_visibility: quill_proto::announcements::CommentVisibility::Public,
        },
    );
    assert!(html.contains(r#"name="sticky" value="on""#));
    assert!(html.contains("Unstick"));
    assert!(html.contains("Make sticky"));
    assert!(!html.contains("Comments aren't available yet"));

    let html = admin_announcement_page(
        &c,
        AdminAnnouncementView {
            title: sticky.title.clone(),
            body: sticky.body.clone(),
            announcement: sticky,
            error: None,
        },
    );
    assert!(html.contains("Sticky: its banner stays at the top of the board."));
    assert!(html.contains(r#"/sticky""#));
}

#[test]
fn listings_tag_sticky_and_closed_announcements() {
    let mut a = announcement("Rules updated", false);
    a.sticky = true;
    a.comments_open = false;
    let html = announcements_page(
        &chrome(false, false),
        AnnouncementsView {
            announcements: vec![a.clone(), announcement("Welcome", false)],
            next_cursor: None,
        },
    );
    assert_eq!(
        html.matches(r#"class="flag flag-sticky">Sticky<"#).count(),
        1
    );
    assert_eq!(html.matches(r#"class="flag">Comments closed<"#).count(), 1);
    let page = announcement_page(&chrome(false, false), announcement_view(a.clone()));
    assert!(page.contains(r#"class="flag flag-sticky">Sticky<"#));

    // Forms on an announcement's edit page lead back to it; the same
    // buttons in the Announcements tab's table don't.
    let c = chrome(true, true);
    let edit = admin_announcement_page(
        &c,
        AdminAnnouncementView {
            title: a.title.clone(),
            body: a.body.clone(),
            announcement: a.clone(),
            error: None,
        },
    );
    assert_eq!(edit.matches(r#"name="back" value="edit""#).count(), 2);
    assert!(edit.contains("View announcement"));
    let list = admin_announcements_page(
        &c,
        AdminAnnouncementsView {
            announcements: vec![a],
            next_cursor: None,
            title: String::new(),
            body: String::new(),
            comments_open: true,
            sticky: false,
            error: None,
            comment_visibility: quill_proto::announcements::CommentVisibility::Public,
        },
    );
    assert!(!list.contains(r#"name="back""#));
}

fn staff_comment(id: u128, staff_name: Option<&str>) -> quill_proto::announcements::Comment {
    use quill_proto::announcements::CommentVoice;
    quill_proto::announcements::Comment {
        voice: if staff_name.is_some() {
            CommentVoice::Staff
        } else {
            CommentVoice::Administrators
        },
        author: None,
        staff_name: staff_name.map(str::to_owned),
        ..comment(id, 0, "unused", false)
    }
}

#[test]
fn staff_comments_are_marked_and_only_admins_speak_as_staff() {
    let mut view = announcement_view(announcement("Welcome", false));
    view.comments = vec![
        staff_comment(200, None),
        staff_comment(201, Some("Nathan")),
        comment(202, 1, "ink_fox", false),
    ];

    // Readers see both staff voices marked in red, with no Report link.
    let html = announcement_page(&chrome(true, false), view.clone());
    assert_eq!(html.matches(r#"class="comment staff""#).count(), 2);
    assert!(html.contains(r#"<span class="staff-name">Administrators</span>"#));
    assert!(html.contains(r#"<span class="staff-name">Nathan</span>"#));
    assert_eq!(html.matches(r#"class="flag flag-admin""#).count(), 2);
    assert!(!html.contains("/comments/00000000-0000-0000-0000-0000000000c8/report"));
    assert!(html.contains("/comments/00000000-0000-0000-0000-0000000000ca/report"));
    // A writer with no personas can't post, and isn't offered staff voices.
    assert!(!html.contains(r#"value="administrators""#));
    assert!(html.contains("Create one"));

    // An administrator without a staff name gets the shared voice and a
    // pointer to Settings; with one, their name too. They can delete staff
    // comments but not remove or report them.
    let html = announcement_page(&chrome(true, true), view.clone());
    assert!(html.contains(r#"value="administrators""#));
    assert!(!html.contains(r#"value="staff""#));
    assert!(html.contains("set a staff name"));
    assert!(html.contains("/comments/00000000-0000-0000-0000-0000000000c8/delete"));
    assert!(!html.contains("/admin/comments/00000000-0000-0000-0000-0000000000c8/remove"));
    assert!(html.contains("/admin/comments/00000000-0000-0000-0000-0000000000ca/remove"));
    view.staff_name = Some("Nathan".into());
    view.speak_as = Some(SPEAK_AS_STAFF.into());
    let html = announcement_page(&chrome(true, true), view);
    assert!(html.contains(r#"value="staff" selected"#));
    assert!(html.contains("Nathan (your staff name, shown as staff)"));
    assert!(!html.contains("set a staff name"));
}

mod moderation_pages {
    use quill_proto::automod::{
        ChainStep, DryRunDecision, DryRunResult, DryRunStep, Hook, ModerationChain, ModerationData,
        ModerationDataVersion, ModerationScript, OnError, ScriptTemplate,
    };

    use super::*;

    fn script(name: &str, log_only: bool) -> ModerationScript {
        ModerationScript {
            id: Uuid::from_u128(7),
            name: name.into(),
            source: "if post.body.contains(\"<b>\") { reject(\"No bold & no tags\") }".into(),
            source_hash: "ab".repeat(32),
            enabled: true,
            log_only,
            data: vec!["blocked_words".into()],
            chains: vec![Hook::PromptCreated, Hook::CommentCreated],
            uploaded_by: Some("boss".into()),
            created_at: datetime!(2026-10-26 12:00 UTC),
            updated_at: datetime!(2026-10-26 12:00 UTC),
        }
    }

    pub(super) fn scripts_view() -> ModerationScriptsView {
        ModerationScriptsView {
            scripts: vec![script("Words", false), script("Trial", true)],
            templates: vec![ScriptTemplate {
                id: "shouting".into(),
                name: "Shouting".into(),
                summary: "Flags posts in capitals.".into(),
                hooks: vec![Hook::PromptCreated],
                source: "let caps = 0.7;".into(),
                data: vec![],
            }],
            ..ModerationScriptsView::default()
        }
    }

    pub(super) fn chains_view() -> ModerationChainsView {
        ModerationChainsView {
            chains: Hook::ALL
                .iter()
                .map(|h| ModerationChain {
                    hook: *h,
                    on_error: if *h == Hook::PromptCreated {
                        OnError::Flag
                    } else {
                        OnError::Continue
                    },
                    steps: if *h == Hook::PromptCreated {
                        vec![ChainStep {
                            script_id: Uuid::from_u128(7),
                            name: "Words".into(),
                            enabled: true,
                            log_only: false,
                        }]
                    } else {
                        vec![]
                    },
                })
                .collect(),
            scripts: vec![script("Words", false)],
            error: None,
        }
    }

    pub(super) fn steps() -> DryRunResult {
        DryRunResult {
            steps: vec![
                DryRunStep {
                    script_id: Uuid::from_u128(1),
                    name: "Trial".into(),
                    decision: DryRunDecision::Flag,
                    detail: Some("Looks odd".into()),
                    category: Some("spam".into()),
                    printed: vec!["checking".into()],
                    micros: 40,
                    log_only: true,
                    decides: false,
                },
                DryRunStep {
                    script_id: Uuid::from_u128(2),
                    name: "Words".into(),
                    decision: DryRunDecision::Reject,
                    detail: Some("Not here".into()),
                    category: None,
                    printed: vec![],
                    micros: 12,
                    log_only: false,
                    decides: true,
                },
            ],
        }
    }

    pub(super) fn data_view() -> ModerationDataView {
        ModerationDataView {
            entry: ModerationData {
                name: "blocked_words".into(),
                description: "Words to refuse".into(),
                content: "# notes\nspam".into(),
                version: 3,
                updated_at: datetime!(2026-10-26 12:00 UTC),
                updated_by: Some("boss".into()),
                used_by: vec!["Words".into()],
            },
            history: (1..=3)
                .rev()
                .map(|v| ModerationDataVersion {
                    version: v,
                    size: 20,
                    created_at: datetime!(2026-10-26 12:00 UTC),
                    created_by: Some("boss".into()),
                })
                .collect(),
            description: "Words to refuse".into(),
            content: "# notes\nspam\nscam".into(),
            base_version: 3,
            preview_title: "A deal".into(),
            preview_body: "s c a m".into(),
            preview: Some(vec![(Hook::PromptCreated, steps())]),
            viewing: None,
            error: None,
        }
    }

    #[test]
    fn scripts_page_lists_modes_chains_and_templates() {
        let html = moderation_scripts_page(&chrome(true, true), scripts_view());
        assert!(html.contains(r#"aria-current="page">Moderation"#));
        assert!(html.contains("Log only") && html.contains("A comment is posted"));
        assert!(html.contains(r#"action="/admin/moderation/templates/shouting""#));
        assert!(html.contains("blob/main/docs/automod.md"));
    }

    #[test]
    fn script_source_is_escaped() {
        let view = ModerationScriptView {
            script: script("Words", false),
            name: "Words".into(),
            source: "reject(\"<b>&</b>\")".into(),
            error: Some("line 1: oops".into()),
        };
        let html = moderation_script_page(&chrome(true, true), view);
        let at = html.find("name=\"source\"").unwrap();
        assert!(
            html[at..].contains("&#60;b&#62;&#38;&#60;/b&#62;"),
            "{}",
            &html[at..at + 200]
        );
        assert!(!html.contains("<b>&</b>"));
        assert!(html.contains("line 1: oops"));
    }

    #[test]
    fn chains_page_numbers_the_scripts_in_each_chain() {
        let html = moderation_chains_page(&chrome(true, true), chains_view());
        assert!(html.contains(r#"value="1""#));
        assert!(html.contains(r#"value="flag" checked"#));
    }

    #[test]
    fn data_page_shows_the_preview_and_history() {
        let html = moderation_data_page(&chrome(true, true), data_view());
        assert!(html.contains("What would happen with your changes"));
        assert!(html.contains("decision-reject") && html.contains("This decides."));
        assert!(html.contains("Save as version 4"));
        assert!(html.contains("Go back to this version"));
        // In use, so there's no delete form.
        assert!(!html.contains("/delete"));
    }

    #[test]
    #[ignore = "writes HTML previews for looking at the pages by eye"]
    fn write_previews() {
        let Some(dir) = std::env::var_os("QUILL_PREVIEW_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let css = format!("<style>{}</style>", crate::STYLESHEET);
        let c = chrome(true, true);
        let pages = [
            ("scripts", moderation_scripts_page(&c, scripts_view())),
            ("chains", moderation_chains_page(&c, chains_view())),
            ("data", moderation_data_page(&c, data_view())),
            (
                "dry-run",
                moderation_dry_run_page(
                    &c,
                    ModerationDryRunView {
                        scripts: vec![script("Words", false)],
                        hook: Hook::PromptCreated,
                        script: None,
                        prompt: String::new(),
                        title: "A deal".into(),
                        body: "s p a m".into(),
                        result: Some(steps()),
                        error: None,
                    },
                ),
            ),
        ];
        for (name, html) in pages {
            let html = html.replace(
                r#"<link rel="stylesheet" href="/static/board.css?v=0.1.0"/>"#,
                &css,
            );
            std::fs::write(dir.join(format!("{name}.html")), html).unwrap();
        }
    }
}

mod rules_pages {
    use quill_proto::rules::{Rules, RulesAcceptance};

    use super::*;

    fn rules(must_accept: bool) -> Rules {
        Rules {
            body: "# Be kind".into(),
            body_html: "<h3>Be kind</h3><p>No <em>spam</em>.</p>".into(),
            version: 3,
            updated_at: datetime!(2026-10-27 12:00 UTC),
            must_accept,
        }
    }

    #[test]
    fn rules_page_shows_the_rules_and_asks_only_when_needed() {
        let c = Chrome {
            current: Nav::Rules,
            ..chrome(true, false)
        };
        let page = rules_page(
            &c,
            RulesView {
                rules: Some(rules(false)),
                error: None,
            },
        );
        assert!(page.contains("<h3>Be kind</h3>"));
        assert!(page.contains("Version 3"));
        assert!(!page.contains(ACCEPT_RULES_PATH));

        let page = rules_page(
            &c,
            RulesView {
                rules: Some(rules(true)),
                error: None,
            },
        );
        assert!(page.contains(&format!(r#"action="{ACCEPT_RULES_PATH}""#)));
        assert!(page.contains(r#"name="version" value="3""#));
        assert!(page.contains("csrf-token-123"));
        // The banner isn't shown on the page it points to.
        assert!(!page.contains("rules-banner"));

        let empty = rules_page(
            &chrome(false, false),
            RulesView {
                rules: None,
                error: None,
            },
        );
        assert!(empty.contains("haven&#39;t written any rules"));
    }

    #[test]
    fn banner_and_footer_link() {
        let mut c = chrome(true, false);
        c.has_rules = true;
        if let Some(v) = c.viewer.as_mut() {
            v.must_accept_rules = true;
        }
        let message = |c: &Chrome| {
            message_page(
                c,
                MessageView {
                    title: "Hi".into(),
                    text: "Hello".into(),
                    link: None,
                },
            )
        };
        let page = message(&c);
        assert!(page.contains("rules-banner"));
        assert!(page.contains(r#"href="/rules""#));

        let plain = message(&chrome(false, false));
        assert!(!plain.contains("rules-banner"));
        assert!(!plain.contains("Server rules"));
    }

    #[test]
    fn sign_up_asks_to_accept_when_the_server_does() {
        let mut c = chrome(false, false);
        c.has_rules = true;
        let register = |accept_rules| {
            auth_page(
                &c,
                AuthView {
                    mode: AuthMode::Register,
                    username: String::new(),
                    error: None,
                    registration_open: true,
                    accept_rules,
                },
            )
        };
        let asks = register(Some(3));
        assert!(asks.contains(r#"name="accept_rules""#));
        assert!(asks.contains(r#"name="rules_version" value="3""#));

        let links = register(None);
        assert!(!links.contains("accept_rules"));
        assert!(links.contains("read the"));
    }

    #[test]
    fn admin_editor() {
        let c = Chrome {
            current: Nav::ServerSettings,
            ..chrome(true, true)
        };
        let first = admin_rules_page(
            &c,
            AdminRulesView {
                current: None,
                body: String::new(),
                minor: false,
                preview_html: None,
                acceptance: RulesAcceptance::None,
                error: None,
            },
        );
        assert!(first.contains("Write the rules"));
        // There's nothing to make a minor edit to yet.
        assert!(!first.contains(r#"name="minor""#));

        let editing = admin_rules_page(
            &c,
            AdminRulesView {
                current: Some(rules(false)),
                body: "# Be kind".into(),
                minor: true,
                preview_html: Some("<h3>Be kind</h3>".into()),
                acceptance: RulesAcceptance::OnChange,
                error: None,
            },
        );
        assert!(editing.contains("now version 3"));
        assert!(editing.contains(r#"name="minor""#));
        assert!(editing.contains(r#"aria-label="Preview""#));
        assert!(editing.contains(r#"value="on-change" checked"#));
        assert!(editing.contains(r#"href="/admin/settings/rules" aria-current="page""#));
    }
}
