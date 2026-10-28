//! The server-rendered prompt board.
//!
//! Plain HTML pages and forms, rendered with `quill-board-ui`. No JavaScript
//! is served, so the board works in Tor Browser's safest mode. Every page
//! calls the same service functions as the JSON API.

mod announcements;
mod comments;
mod handlers;
mod moderation;
mod picker;
mod rules;
pub mod session;
mod settings;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use quill_board_ui::{Chrome, MessageView, Nav, Viewer};

use crate::{AppState, error::ApiError};
use session::Session;
use sha2::{Digest, Sha256};

/// Board forms may carry a 20,000-character prompt, which URL-encoding can
/// triple in size.
const FORM_BODY_LIMIT: usize = 256 * 1024;

/// No scripts, no third-party anything, no framing.
const CSP: &str = "default-src 'none'; style-src 'self'; img-src 'self'; \
                   form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::board))
        .route("/p/{id}", get(handlers::prompt))
        .route(
            "/p/{id}/edit",
            get(handlers::edit_form).post(handlers::edit_submit),
        )
        .route("/p/{id}/state", post(handlers::set_state))
        .route("/p/{id}/delete", post(handlers::delete))
        .route(
            "/p/{id}/report",
            get(handlers::report_form).post(handlers::report_submit),
        )
        .route(quill_board_ui::RULES_PATH, get(rules::page))
        .route(quill_board_ui::ACCEPT_RULES_PATH, post(rules::accept))
        .route("/announcements", get(announcements::list))
        .route(quill_board_ui::DISMISS_PATH, post(announcements::dismiss))
        .route("/announcements/{id}", get(announcements::show))
        .route("/announcements/{id}/comments", post(comments::post))
        .route("/comments/{id}", get(comments::goto))
        .route("/comments/{id}/delete", post(comments::delete))
        .route(
            "/comments/{id}/report",
            get(comments::report_form).post(comments::report_submit),
        )
        .route("/admin/comments/{id}/remove", post(comments::admin_remove))
        .route("/new", get(handlers::new_form).post(handlers::new_submit))
        .route("/mine", get(handlers::mine))
        .route(
            "/personas",
            get(handlers::personas).post(handlers::persona_create),
        )
        .route("/personas/{id}/delete", post(handlers::persona_delete))
        .route("/notices", get(handlers::notices))
        .route("/more", get(handlers::more))
        .route(
            "/settings",
            get(handlers::settings).post(handlers::settings_save),
        )
        .route(
            "/login",
            get(handlers::login_form).post(handlers::login_submit),
        )
        .route(
            "/register",
            get(handlers::register_form).post(handlers::register_submit),
        )
        .route("/logout", post(handlers::logout))
        .route("/admin/reports", get(handlers::admin_reports))
        .route("/admin/reports/{id}/resolve", post(handlers::admin_resolve))
        .route("/admin/log", get(handlers::admin_log))
        .route("/admin/log/{id}", get(handlers::admin_log_entry))
        .route("/admin/moderation", get(moderation::index))
        .route(
            "/admin/moderation/scripts",
            get(moderation::scripts_page).post(moderation::upload),
        )
        .route(
            "/admin/moderation/scripts/{id}",
            get(moderation::script_page).post(moderation::script_save),
        )
        .route(
            "/admin/moderation/scripts/{id}/mode",
            post(moderation::script_mode),
        )
        .route(
            "/admin/moderation/scripts/{id}/delete",
            post(moderation::script_delete),
        )
        .route(
            "/admin/moderation/templates/{id}",
            post(moderation::use_template),
        )
        .route("/admin/moderation/chains", get(moderation::chains_page))
        .route(
            "/admin/moderation/chains/{hook}",
            post(moderation::chain_save),
        )
        .route(
            "/admin/moderation/data",
            get(moderation::data_list_page).post(moderation::data_create),
        )
        .route(
            "/admin/moderation/data/{name}",
            get(moderation::data_page).post(moderation::data_save),
        )
        .route(
            "/admin/moderation/data/{name}/restore",
            post(moderation::data_restore),
        )
        .route(
            "/admin/moderation/data/{name}/delete",
            post(moderation::data_delete),
        )
        .route(
            "/admin/moderation/dry-run",
            get(moderation::dry_run_page).post(moderation::dry_run_submit),
        )
        .route("/admin/settings", get(settings::index))
        .route(
            "/admin/settings/tag-types",
            get(settings::tag_types).post(settings::tag_type_create),
        )
        .route(
            "/admin/settings/tag-types/{name}",
            post(settings::tag_type_update),
        )
        .route(
            "/admin/settings/tag-types/{name}/limits",
            post(settings::tag_type_limits),
        )
        .route(
            "/admin/settings/tag-types/{name}/outside",
            get(settings::tag_type_outside),
        )
        .route("/admin/settings/new-tags", post(settings::new_tags_save))
        .route("/admin/settings/sessions", get(settings::sessions_page))
        .route(
            "/admin/settings/attachments",
            post(settings::attachments_save),
        )
        .route(
            "/admin/settings/announcements",
            get(announcements::admin_list).post(announcements::admin_post),
        )
        .route(
            "/admin/settings/announcements/{id}",
            get(announcements::admin_edit_form).post(announcements::admin_edit),
        )
        .route(
            "/admin/settings/announcements/{id}/comments",
            post(announcements::admin_comments),
        )
        .route(
            "/admin/settings/announcements/{id}/delete",
            post(announcements::admin_delete),
        )
        .route(
            "/admin/settings/announcements/{id}/sticky",
            post(announcements::admin_sticky),
        )
        .route(
            "/admin/settings/comment-visibility",
            post(announcements::admin_comment_visibility),
        )
        .route(
            quill_board_ui::ADMIN_RULES_PATH,
            get(rules::admin_page).post(rules::admin_save),
        )
        .route(
            "/admin/settings/rules-acceptance",
            post(rules::admin_acceptance),
        )
        .route("/admin/settings/tag-requests", get(settings::tag_requests))
        .route(
            "/admin/settings/handles",
            get(settings::handles).post(settings::handles_reserve),
        )
        .route(
            "/admin/settings/handles/{kind}/{pattern}/remove",
            post(settings::handle_remove),
        )
        .route(
            "/admin/settings/tags/{namespace}/{slug}/status",
            post(settings::tag_status),
        )
        .route(
            "/admin/settings/tags",
            get(settings::tags).post(settings::tag_create),
        )
        .route(
            "/admin/settings/tags/{namespace}/{slug}",
            get(settings::tag).post(settings::tag_rename),
        )
        .route(
            "/admin/settings/tags/{namespace}/{slug}/merge",
            post(settings::tag_merge),
        )
        .route(
            "/admin/settings/tags/{namespace}/{slug}/color",
            post(settings::tag_color),
        )
        .route(
            "/admin/settings/tags/{namespace}/{slug}/delete",
            get(settings::tag_delete_form).post(settings::tag_delete),
        )
        .route(quill_board_ui::STYLESHEET_PATH, get(stylesheet))
        .route(quill_board_ui::chips::TAG_COLORS_PATH, get(tag_colors))
        // No icon; answering keeps browsers from logging a 404 on every page.
        .route("/favicon.ico", get(|| async { StatusCode::NO_CONTENT }))
        .fallback(not_found)
        .layer(middleware::from_fn(reject_cross_site_posts))
        .layer(middleware::from_fn(security_headers))
        .layer(DefaultBodyLimit::max(FORM_BODY_LIMIT))
}

async fn stylesheet() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        quill_board_ui::STYLESHEET,
    )
}

/// Chip colours from Server settings, as a stylesheet from our own origin
/// (the CSP allows no inline styles). Browsers revalidate it on each page
/// and get "not modified" until a colour changes.
async fn tag_colors(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let built = async {
        let namespaces = crate::prompts::namespaces(&state).await?;
        // Only visible tags: the sheet is public and mustn't reveal others.
        let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT namespace, slug, chip_bg, chip_fg FROM tags
             WHERE instance = '' AND canonical_id IS NULL AND status = 'visible'
               AND chip_bg IS NOT NULL
             ORDER BY namespace, slug",
        )
        .fetch_all(&state.pool)
        .await?;
        let tags: Vec<_> = rows
            .into_iter()
            .map(|(ns, slug, bg, fg)| {
                let color = quill_proto::board::ChipColor {
                    background: bg,
                    text: fg,
                };
                (ns, slug, color)
            })
            .collect();
        Ok::<_, ApiError>(quill_board_ui::chips::tag_color_css(&namespaces, &tags))
    }
    .await;
    let css = match built {
        Ok(css) => css,
        Err(e) => {
            tracing::error!(error = ?e, "building tag colours");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let digest = Sha256::digest(css.as_bytes());
    let etag = format!(
        "\"{}\"",
        digest
            .iter()
            .take(12)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag));
    let common = [
        (header::CACHE_CONTROL, "no-cache".to_owned()),
        (header::ETAG, etag),
    ];
    if matches {
        return (StatusCode::NOT_MODIFIED, common).into_response();
    }
    (
        common,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        css,
    )
        .into_response()
}

async fn not_found(State(state): State<AppState>) -> Response {
    message(
        &chrome(&state, None, Nav::Other, None),
        StatusCode::NOT_FOUND,
        "Page not found",
        "There's nothing at this address. The prompt may have been deleted.",
    )
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Never leak addresses to other sites. ("no-referrer" would also make
    // browsers send `Origin: null` on our own forms.)
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if !h.contains_key(header::CACHE_CONTROL) {
        // Pages can carry a CSRF token; never cache them.
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

/// Refuses form posts that a browser says came from another site. This
/// protects anonymous forms (sign-in, registration, reports) as well as the
/// signed-in ones that also carry a CSRF token.
async fn reject_cross_site_posts(request: Request, next: Next) -> Response {
    if request.method() == Method::POST && is_cross_site(request.headers()) {
        return (
            StatusCode::FORBIDDEN,
            "Forms on this site can only be sent from this site.",
        )
            .into_response();
    }
    next.run(request).await
}

fn is_cross_site(headers: &HeaderMap) -> bool {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    // Modern browsers say exactly where a request came from.
    if let Some(site) = get("sec-fetch-site") {
        return !matches!(site, "same-origin" | "none");
    }
    // Older browsers: compare the Origin header with the Host.
    if let Some(origin) = get("origin") {
        let origin_host = origin
            .split_once("://")
            .map_or(origin, |(_, rest)| rest)
            .trim_end_matches('/');
        return Some(origin_host) != get("host");
    }
    false
}

// ---- Helpers for handlers ------------------------------------------------

pub(crate) fn chrome(
    state: &AppState,
    session: Option<&Session>,
    current: Nav,
    notice: Option<&str>,
) -> Chrome {
    Chrome {
        site_name: state.config.domain.clone(),
        viewer: session.map(|s| Viewer {
            csrf: s.csrf.clone(),
            is_admin: s.is_admin,
            maturity_max: s.maturity_max,
            pending_tags: s.pending_tags,
            unread_notices: s.unread_notices,
            must_accept_rules: s.must_accept_rules,
        }),
        source_url: state.config.source_url.clone(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        notice: notice.and_then(notice_text).map(str::to_owned),
        current,
        has_rules: state
            .rules_published
            .load(std::sync::atomic::Ordering::Relaxed),
    }
}

/// Fixed messages shown after a redirect, keyed by `?notice=`. Only these
/// strings can appear, so the query parameter can't inject text.
fn notice_text(key: &str) -> Option<&'static str> {
    Some(match key {
        "posted" => "Prompt posted.",
        "saved" => "Changes saved.",
        "status" => "Status updated.",
        "deleted" => "Prompt deleted.",
        "reported" => "Report sent to the server's administrators. Thank you.",
        "signed-in" => "Signed in.",
        "signed-out" => "Signed out.",
        "registered" => "Account created. Create a persona to start posting.",
        "persona-created" => "Persona created.",
        "persona-deleted" => "Persona deleted.",
        "settings" => "Settings saved.",
        "resolved" => "Report resolved.",
        "tag-type-created" => "Tag type added.",
        "tag-type-saved" => "Tag type saved.",
        "tag-created" => "Tag added.",
        "tag-saved" => "Tag renamed.",
        "tag-merged" => "Tag merged.",
        "tag-deleted" => "Tag deleted.",
        "tag-rejected" => "Tag rejected.",
        "tag-approved" => "Tag approved.",
        "tag-hidden" => "Tag hidden.",
        "tag-shown" => "Tag shown.",
        "new-tags-saved" => "Setting saved.",
        "attachments-saved" => "Setting saved.",
        "tag-color" => "Colour saved.",
        "handles-reserved" => "Handles reserved.",
        "handle-removed" => "Handle no longer reserved.",
        "announcement-posted" => "Announcement posted.",
        "announcement-saved" => "Changes saved.",
        "announcement-comments" => "Setting saved.",
        "announcement-sticky" => "Setting saved.",
        "announcement-deleted" => "Announcement deleted.",
        "comment-visibility-saved" => "Setting saved.",
        "comment-posted" => "Comment posted.",
        "comment-deleted" => "Comment deleted.",
        "comment-removed" => "Comment removed. Its author has been told why.",
        "script-uploaded" => "Script uploaded. Add it to a chain to run it.",
        "script-saved" => "Script saved.",
        "script-mode" => "Setting saved.",
        "script-removed" => "Script removed.",
        "chain-saved" => "Chain saved.",
        "data-saved" => "Saved as a new version.",
        "data-restored" => "Saved the old version again, as a new version.",
        "data-deleted" => "Entry deleted.",
        "rules-accepted" => "Thank you for accepting the server rules.",
        "rules-saved" => "Rules saved as a new version.",
        "rules-acceptance-saved" => "Setting saved.",
        _ => return None,
    })
}

pub(crate) fn html(status: StatusCode, page: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(page),
    )
        .into_response()
}

pub(crate) fn message(chrome: &Chrome, status: StatusCode, title: &str, text: &str) -> Response {
    html(
        status,
        quill_board_ui::message_page(
            chrome,
            MessageView {
                title: title.to_owned(),
                text: text.to_owned(),
                link: Some(("/".to_owned(), "Go to the board".to_owned())),
            },
        ),
    )
}

pub(crate) fn redirect(to: &str) -> Response {
    Redirect::to(to).into_response()
}

/// Message shown for errors a form can't fix, such as a missing prompt or a
/// server fault.
pub(crate) fn error_page(chrome: &Chrome, err: ApiError) -> Response {
    match err {
        ApiError::NotFound => message(
            chrome,
            StatusCode::NOT_FOUND,
            "Not found",
            "There's nothing here. It may have been deleted.",
        ),
        ApiError::Unauthorized => redirect("/login"),
        ApiError::Forbidden(text) => message(chrome, StatusCode::FORBIDDEN, "Not allowed", &text),
        ApiError::RulesNotAccepted(text) => html(
            StatusCode::FORBIDDEN,
            quill_board_ui::message_page(
                chrome,
                MessageView {
                    title: "Accept the server rules first".to_owned(),
                    text,
                    link: Some((
                        rules::READ_RULES.0.to_owned(),
                        rules::READ_RULES.1.to_owned(),
                    )),
                },
            ),
        ),
        ApiError::TooManyRequests => message(
            chrome,
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests",
            "Too many of these have been sent recently. Try again in an hour.",
        ),
        ApiError::Internal(e) => {
            tracing::error!(error = ?e, "internal error rendering the board");
            message(
                chrome,
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong",
                "The server couldn't complete that. Try again in a moment.",
            )
        }
        other => message(
            chrome,
            StatusCode::BAD_REQUEST,
            "That didn't work",
            &other.to_string(),
        ),
    }
}

/// Shown when a signed-in form's CSRF token doesn't match the session.
pub(crate) fn csrf_failed(chrome: &Chrome) -> Response {
    message(
        chrome,
        StatusCode::FORBIDDEN,
        "This form has expired",
        "Go back, reload the page, and send the form again.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn cross_site_detection() {
        assert!(!is_cross_site(&headers(&[])));
        assert!(!is_cross_site(&headers(&[(
            "sec-fetch-site",
            "same-origin"
        )])));
        assert!(!is_cross_site(&headers(&[("sec-fetch-site", "none")])));
        assert!(is_cross_site(&headers(&[("sec-fetch-site", "cross-site")])));
        assert!(is_cross_site(&headers(&[("sec-fetch-site", "same-site")])));
        assert!(!is_cross_site(&headers(&[
            ("origin", "https://writers.example"),
            ("host", "writers.example"),
        ])));
        assert!(is_cross_site(&headers(&[
            ("origin", "https://evil.example"),
            ("host", "writers.example"),
        ])));
        assert!(is_cross_site(&headers(&[
            ("origin", "null"),
            ("host", "writers.example")
        ])));
        // Sec-Fetch-Site wins over a privacy-stripped Origin.
        assert!(!is_cross_site(&headers(&[
            ("sec-fetch-site", "same-origin"),
            ("origin", "null"),
            ("host", "writers.example"),
        ])));
    }

    #[test]
    fn only_known_notices_render() {
        assert_eq!(notice_text("posted"), Some("Prompt posted."));
        assert_eq!(notice_text("<script>"), None);
    }
}
