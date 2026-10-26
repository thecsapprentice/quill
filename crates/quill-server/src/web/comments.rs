//! Comment forms on the board: posting under an announcement, deleting
//! one's own, reporting, and an administrator's direct removal.

use axum::{
    Form,
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use quill_board_ui::{CommentReportView, Nav, announcement_url};
use quill_proto::{
    announcements::{Comment, CreateComment, ReportComment},
    board::ReportReason,
};
use serde::Deserialize;
use uuid::Uuid;

use super::{
    announcements::{Draft, render_announcement},
    chrome, csrf_failed, error_page,
    handlers::{form_error, opt},
    html, redirect,
    session::{MaybeSession, Session},
    settings::require_admin,
};
use crate::{AppState, comments, error::ApiError};

/// Where a comment sits: its announcement's page, at the comment.
fn at(announcement: Uuid, comment: Uuid, notice: &str) -> String {
    format!(
        "{}?notice={notice}#c-{comment}",
        announcement_url(announcement)
    )
}

/// `/comments/{id}`: leads to the comment under its announcement.
pub async fn goto(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> Response {
    let c = chrome(&state, session.as_ref(), Nav::Announcements, None);
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    match comments::announcement_of(&state, session.as_ref().map(|s| s.account_id), id).await {
        Ok(a) => redirect(&format!("{}#c-{id}", announcement_url(a))),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CommentForm {
    csrf: Option<String>,
    persona: Option<String>,
    body: Option<String>,
}

pub async fn post(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<CommentForm>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let c = chrome(&state, Some(&s), Nav::Announcements, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    let persona = opt(&form.persona).and_then(|p| p.parse::<Uuid>().ok());
    let body = form.body.clone().unwrap_or_default();
    let again = |error: String| Draft {
        persona,
        body: body.clone(),
        error: Some(error),
    };
    let Some(persona) = persona else {
        let draft = again("Choose a persona to comment as.".into());
        return render_announcement(&state, Some(&s), id, None, None, draft).await;
    };
    let create = CreateComment {
        persona,
        body: body.clone(),
    };
    match comments::post(&state, s.account_id, id, &create).await {
        Ok(comment) => redirect(&at(id, comment.id, "comment-posted")),
        Err(e) => match form_error(&e) {
            Some(m) => render_announcement(&state, Some(&s), id, None, None, again(m)).await,
            None => error_page(&c, e),
        },
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ConfirmForm {
    csrf: Option<String>,
    confirm: Option<String>,
}

pub async fn delete(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<ConfirmForm>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let c = chrome(&state, Some(&s), Nav::Announcements, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    if form.confirm.as_deref() != Some("yes") {
        return error_page(
            &c,
            ApiError::BadRequest("Tick the box to confirm deleting the comment.".into()),
        );
    }
    // Where to go back to, looked up before the comment is gone.
    let back = comments::announcement_of(&state, Some(s.account_id), id).await;
    match comments::delete_own(&state, s.account_id, id).await {
        Ok(()) => match back {
            Ok(a) => redirect(&format!(
                "{}?notice=comment-deleted#comments-title",
                announcement_url(a)
            )),
            Err(_) => redirect("/announcements?notice=comment-deleted"),
        },
        Err(e) => error_page(&c, e),
    }
}

// ---- Reporting ---------------------------------------------------------------

/// A comment anyone may report: visible to everyone and not removed.
async fn reportable(state: &AppState, id: &str) -> Result<Comment, ApiError> {
    let id: Uuid = id.parse().map_err(|_| ApiError::NotFound)?;
    comments::announcement_of(state, None, id).await?;
    match comments::load(&state.pool, id).await? {
        Some(c) if !c.removed => Ok(c),
        _ => Err(ApiError::NotFound),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ReportForm {
    csrf: Option<String>,
    reason: Option<String>,
    details: Option<String>,
}

pub async fn report_form(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> Response {
    let c = chrome(&state, session.as_ref(), Nav::Announcements, None);
    match reportable(&state, &id).await {
        Ok(comment) => html(
            StatusCode::OK,
            quill_board_ui::comment_report_page(
                &c,
                CommentReportView {
                    comment,
                    reason: None,
                    details: String::new(),
                    error: None,
                },
            ),
        ),
        Err(e) => error_page(&c, e),
    }
}

pub async fn report_submit(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<ReportForm>,
) -> Response {
    let c = chrome(&state, session.as_ref(), Nav::Announcements, None);
    if let Some(s) = &session
        && !s.csrf_matches(form.csrf.as_deref())
    {
        return csrf_failed(&c);
    }
    let comment = match reportable(&state, &id).await {
        Ok(comment) => comment,
        Err(e) => return error_page(&c, e),
    };
    let reason = opt(&form.reason).and_then(|r| r.parse::<ReportReason>().ok());
    let rerender = |error: String| {
        html(
            StatusCode::BAD_REQUEST,
            quill_board_ui::comment_report_page(
                &c,
                CommentReportView {
                    comment: comment.clone(),
                    reason,
                    details: form.details.clone().unwrap_or_default(),
                    error: Some(error),
                },
            ),
        )
    };
    let Some(reason) = reason else {
        return rerender("Choose what's wrong with the comment.".into());
    };
    let report = ReportComment {
        reason,
        details: opt(&form.details).map(str::to_owned),
    };
    let reporter = session.as_ref().map(|s| s.account_id);
    match crate::reports::file_comment_report(&state, reporter, comment.id, &report).await {
        Ok(()) => redirect(&at(comment.announcement, comment.id, "reported")),
        Err(ApiError::BadRequest(m)) => rerender(format!("Details {m}.")),
        Err(e) => error_page(&c, e),
    }
}

// ---- Administration ----------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct RemoveForm {
    csrf: Option<String>,
    reason: Option<String>,
}

pub async fn admin_remove(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<RemoveForm>,
) -> Response {
    let s: Session = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::Announcements, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    let Some(reason) = opt(&form.reason).and_then(|r| r.parse::<ReportReason>().ok()) else {
        return error_page(&c, ApiError::BadRequest("Choose a reason.".into()));
    };
    match comments::remove(&state, s.account_id, id, reason).await {
        Ok(()) => match comments::announcement_of(&state, Some(s.account_id), id).await {
            Ok(a) => redirect(&at(a, id, "comment-removed")),
            Err(e) => error_page(&c, e),
        },
        Err(e) => error_page(&c, e),
    }
}
