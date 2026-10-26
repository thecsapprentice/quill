//! Announcement pages: the public list and detail pages, dismissing the
//! board's banner, and the administrators' tab in Server settings.

use axum::{
    Form,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use quill_board_ui::{
    AdminAnnouncementView, AdminAnnouncementsView, AnnouncementView, AnnouncementsView, Nav,
};
use quill_proto::{
    announcements::{
        Announcement, AnnouncementQuery, CommentPage, CommentQuery, CommentVisibility,
        CreateAnnouncement, UpdateAnnouncement,
    },
    board::UpdateServerSettings,
    limits::MAX_PAGE_SIZE,
};
use serde::Deserialize;
use uuid::Uuid;

use super::{
    chrome, csrf_failed, error_page,
    handlers::{form_error, opt},
    html, redirect,
    session::{MaybeSession, Session},
    settings::require_admin,
};
use crate::{AppState, announcements, error::ApiError};

/// Holds the id of the newest announcement the reader dismissed. `__Host-`
/// for the same reasons as the session cookie; it names no account.
const DISMISSED_COOKIE: &str = "__Host-quill_dismissed";

/// How long a dismissal is remembered: a year.
const DISMISSED_MAX_AGE: u32 = 365 * 24 * 60 * 60;

fn dismissed(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == DISMISSED_COOKIE)
        .and_then(|(_, value)| value.parse().ok())
}

/// The board's banners: every sticky announcement, which can't be
/// dismissed, and the newest other one, unless this reader dismissed it or
/// a newer one. Ids are UUIDv7, so they order by posting time: deleting the
/// newest doesn't bring back one already dismissed.
pub(super) async fn banners(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(Vec<Announcement>, Option<Announcement>), ApiError> {
    let (sticky, newest) = announcements::banners(state).await?;
    let seen = dismissed(headers);
    Ok((sticky, newest.filter(|a| seen.is_none_or(|s| a.id > s))))
}

#[derive(Debug, Default, Deserialize)]
pub struct DismissForm {
    id: Option<String>,
}

/// Remembers the dismissed banner in a cookie, so it needs no account. No
/// CSRF token: anyone may send it, and the router already refuses
/// cross-site posts, so another site can't hide the banner.
pub async fn dismiss(Form(form): Form<DismissForm>) -> Response {
    let Some(id) = opt(&form.id).and_then(|i| i.parse::<Uuid>().ok()) else {
        return redirect("/");
    };
    let cookie = format!(
        "{DISMISSED_COOKIE}={id}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={DISMISSED_MAX_AGE}"
    );
    match HeaderValue::from_str(&cookie) {
        Ok(v) => ([(header::SET_COOKIE, v)], Redirect::to("/")).into_response(),
        Err(_) => redirect("/"),
    }
}

// ---- Public pages ------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct ListParams {
    cursor: Option<String>,
    notice: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(p): Query<ListParams>,
) -> Response {
    let c = chrome(&state, session.as_ref(), Nav::Announcements, opt(&p.notice));
    let query = AnnouncementQuery {
        cursor: opt(&p.cursor).map(str::to_owned),
        limit: None,
        sticky: None,
    };
    match announcements::list(&state, &query).await {
        Ok(page) => html(
            StatusCode::OK,
            quill_board_ui::announcements_page(
                &c,
                AnnouncementsView {
                    announcements: page.items,
                    next_cursor: page.next_cursor,
                },
            ),
        ),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ShowParams {
    /// Comments after this cursor.
    after: Option<String>,
    notice: Option<String>,
}

/// The comment form's values, kept when it's sent back with an error.
#[derive(Debug, Default)]
pub(super) struct Draft {
    pub persona: Option<Uuid>,
    pub body: String,
    pub error: Option<String>,
}

/// An announcement with a page of its comments and the comment form.
pub(super) async fn render_announcement(
    state: &AppState,
    session: Option<&Session>,
    id: Uuid,
    after: Option<&str>,
    notice: Option<&str>,
    draft: Draft,
) -> Response {
    let c = chrome(state, session, Nav::Announcements, notice);
    let viewer = session.map(|s| s.account_id);
    let loaded = async {
        let announcement = announcements::get(state, id).await?;
        let query = CommentQuery {
            cursor: after.map(str::to_owned),
            limit: Some(MAX_PAGE_SIZE),
        };
        // Hidden comments aren't an error here: the page says to sign in.
        let readable = crate::comments::readable_by(state, viewer).await?;
        let page = if readable {
            crate::comments::list(state, viewer, id, &query).await?
        } else {
            CommentPage {
                items: Vec::new(),
                next_cursor: None,
            }
        };
        let personas = match viewer {
            Some(account) => crate::routes::personas(state, account).await?,
            None => Vec::new(),
        };
        Ok::<_, ApiError>((announcement, page, personas, !readable))
    }
    .await;
    let (announcement, page, personas, comments_hidden) = match loaded {
        Ok(l) => l,
        Err(e) => return error_page(&c, e),
    };
    let status = if draft.error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    let more_comments = page.next_cursor.map(|cursor| {
        format!(
            "{}?after={cursor}#comments-title",
            quill_board_ui::announcement_url(id)
        )
    });
    html(
        status,
        quill_board_ui::announcement_page(
            &c,
            AnnouncementView {
                announcement,
                comments: page.items,
                more_comments,
                personas,
                draft: draft.body,
                persona: draft.persona,
                error: draft.error,
                comments_hidden,
            },
        ),
    )
}

pub async fn show(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Query(p): Query<ShowParams>,
) -> Response {
    let Ok(id) = id.parse::<Uuid>() else {
        let c = chrome(&state, session.as_ref(), Nav::Announcements, None);
        return error_page(&c, ApiError::NotFound);
    };
    render_announcement(
        &state,
        session.as_ref(),
        id,
        opt(&p.after),
        opt(&p.notice),
        Draft::default(),
    )
    .await
}

// ---- Administration ----------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct AnnouncementForm {
    csrf: Option<String>,
    title: Option<String>,
    body: Option<String>,
    /// The "Allow comments" box: present only when ticked.
    comments: Option<String>,
    /// The "Sticky" box: present only when ticked.
    sticky: Option<String>,
}

async fn render_admin_list(
    state: &AppState,
    s: &Session,
    params: &ListParams,
    form: Option<&AnnouncementForm>,
    error: Option<String>,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, opt(&params.notice));
    let query = AnnouncementQuery {
        cursor: opt(&params.cursor).map(str::to_owned),
        limit: None,
        sticky: None,
    };
    let loaded = async {
        let page = announcements::list(state, &query).await?;
        let settings = crate::server_settings::settings(state).await?;
        Ok::<_, ApiError>((page, settings.comment_visibility))
    }
    .await;
    let (page, comment_visibility) = match loaded {
        Ok(l) => l,
        Err(e) => return error_page(&c, e),
    };
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    html(
        status,
        quill_board_ui::admin_announcements_page(
            &c,
            AdminAnnouncementsView {
                announcements: page.items,
                next_cursor: page.next_cursor,
                title: form.and_then(|f| f.title.clone()).unwrap_or_default(),
                body: form.and_then(|f| f.body.clone()).unwrap_or_default(),
                // A new form starts with comments allowed.
                comments_open: form.is_none_or(|f| f.comments.is_some()),
                sticky: form.is_some_and(|f| f.sticky.is_some()),
                error,
                comment_visibility,
            },
        ),
    )
}

pub async fn admin_list(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(p): Query<ListParams>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_admin_list(&state, &s, &p, None, None).await,
        Err(r) => r,
    }
}

pub async fn admin_post(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<AnnouncementForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::ServerSettings, None));
    }
    let body = CreateAnnouncement {
        title: form.title.clone().unwrap_or_default(),
        body: form.body.clone().unwrap_or_default(),
        comments_open: form.comments.is_some(),
        sticky: form.sticky.is_some(),
    };
    match announcements::post(&state, s.account_id, &body).await {
        Ok(a) => redirect(&format!(
            "/announcements/{}?notice=announcement-posted",
            a.id
        )),
        Err(e) => match form_error(&e) {
            Some(m) => {
                render_admin_list(&state, &s, &ListParams::default(), Some(&form), Some(m)).await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

async fn render_admin_edit(
    state: &AppState,
    s: &Session,
    id: &str,
    form: Option<&AnnouncementForm>,
    error: Option<String>,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, None);
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    let a = match announcements::get(state, id).await {
        Ok(a) => a,
        Err(e) => return error_page(&c, e),
    };
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    html(
        status,
        quill_board_ui::admin_announcement_page(
            &c,
            AdminAnnouncementView {
                title: form
                    .and_then(|f| f.title.clone())
                    .unwrap_or_else(|| a.title.clone()),
                body: form
                    .and_then(|f| f.body.clone())
                    .unwrap_or_else(|| a.body.clone()),
                announcement: a,
                error,
            },
        ),
    )
}

pub async fn admin_edit_form(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_admin_edit(&state, &s, &id, None, None).await,
        Err(r) => r,
    }
}

pub async fn admin_edit(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<AnnouncementForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(uuid) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    let body = UpdateAnnouncement {
        title: Some(form.title.clone().unwrap_or_default()),
        body: Some(form.body.clone().unwrap_or_default()),
        comments_open: None,
        sticky: None,
    };
    match announcements::update(&state, s.account_id, uuid, &body).await {
        Ok(a) => redirect(&format!(
            "/announcements/{}?notice=announcement-saved",
            a.id
        )),
        Err(e) => match form_error(&e) {
            Some(m) => render_admin_edit(&state, &s, &id, Some(&form), Some(m)).await,
            None => error_page(&c, e),
        },
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct VisibilityForm {
    csrf: Option<String>,
    visibility: Option<String>,
}

/// The server-wide "who can read comments" setting.
pub async fn admin_comment_visibility(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<VisibilityForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Some(visibility) = opt(&form.visibility).and_then(|v| v.parse::<CommentVisibility>().ok())
    else {
        return error_page(&c, ApiError::BadRequest("Choose a setting.".into()));
    };
    let body = UpdateServerSettings {
        comment_visibility: Some(visibility),
        ..UpdateServerSettings::default()
    };
    match crate::server_settings::update(&state, s.account_id, &body).await {
        Ok(_) => redirect("/admin/settings/announcements?notice=comment-visibility-saved"),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CommentsForm {
    csrf: Option<String>,
    comments: Option<String>,
}

pub async fn admin_comments(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<CommentsForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    let open = match opt(&form.comments) {
        Some("on") => true,
        Some("off") => false,
        _ => return error_page(&c, ApiError::BadRequest("Choose a setting.".into())),
    };
    let body = UpdateAnnouncement {
        comments_open: Some(open),
        ..UpdateAnnouncement::default()
    };
    match announcements::update(&state, s.account_id, id, &body).await {
        Ok(_) => redirect("/admin/settings/announcements?notice=announcement-comments"),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct StickyForm {
    csrf: Option<String>,
    sticky: Option<String>,
}

pub async fn admin_sticky(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<StickyForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    let sticky = match opt(&form.sticky) {
        Some("on") => true,
        Some("off") => false,
        _ => return error_page(&c, ApiError::BadRequest("Choose a setting.".into())),
    };
    let body = UpdateAnnouncement {
        sticky: Some(sticky),
        ..UpdateAnnouncement::default()
    };
    match announcements::update(&state, s.account_id, id, &body).await {
        Ok(_) => redirect("/admin/settings/announcements?notice=announcement-sticky"),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct DeleteForm {
    csrf: Option<String>,
    confirm: Option<String>,
}

pub async fn admin_delete(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<DeleteForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    if form.confirm.as_deref() != Some("yes") {
        return error_page(
            &c,
            ApiError::BadRequest("Tick the box to confirm deleting the announcement.".into()),
        );
    }
    match announcements::delete(&state, s.account_id, id).await {
        Ok(()) => redirect("/admin/settings/announcements?notice=announcement-deleted"),
        Err(e) => error_page(&c, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_dismissed_cookie() {
        let id = Uuid::now_v7();
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_str(&format!("a=1; {DISMISSED_COOKIE}={id}")).unwrap(),
        );
        assert_eq!(dismissed(&h), Some(id));
        let mut bad = HeaderMap::new();
        bad.append(
            header::COOKIE,
            HeaderValue::from_static("__Host-quill_dismissed=not-a-uuid"),
        );
        assert_eq!(dismissed(&bad), None);
        assert_eq!(dismissed(&HeaderMap::new()), None);
    }
}
