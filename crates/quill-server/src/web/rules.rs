//! The server rules on the board: the public page, accepting them, and the
//! administrators' editor in Server settings.

use axum::{
    Form,
    extract::{Query, State},
    http::StatusCode,
    response::Response,
};
use quill_board_ui::{ADMIN_RULES_PATH, AdminRulesView, Nav, RULES_PATH, RulesView};
use quill_proto::{board::UpdateServerSettings, rules::UpdateRules};
use serde::Deserialize;

use super::{
    chrome, csrf_failed, error_page,
    handlers::{form_error, opt},
    html, redirect,
    session::MaybeSession,
    settings::{NoticeQuery, require_admin},
};
use crate::{AppState, error::ApiError, markdown, rules, server_settings};

pub async fn page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    let c = chrome(&state, session.as_ref(), Nav::Rules, q.notice());
    match rules::get(&state, session.as_ref().map(|s| s.account_id)).await {
        Ok(found) => html(
            StatusCode::OK,
            quill_board_ui::rules_page(
                &c,
                RulesView {
                    rules: found,
                    error: None,
                },
            ),
        ),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct AcceptForm {
    csrf: Option<String>,
    version: Option<String>,
}

pub async fn accept(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<AcceptForm>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let c = chrome(&state, Some(&s), Nav::Rules, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Some(version) = opt(&form.version).and_then(|v| v.parse().ok()) else {
        return error_page(&c, ApiError::BadRequest("Which version?".into()));
    };
    match rules::accept(&state, s.account_id, version).await {
        Ok(()) => redirect("/?notice=rules-accepted"),
        // The rules changed while they read: show the new ones.
        Err(e @ ApiError::Conflict(_)) => {
            let found = rules::get(&state, Some(s.account_id)).await.ok().flatten();
            html(
                StatusCode::CONFLICT,
                quill_board_ui::rules_page(
                    &c,
                    RulesView {
                        rules: found,
                        error: form_error(&e),
                    },
                ),
            )
        }
        Err(e) => error_page(&c, e),
    }
}

/// The editor, with `body` and `minor` as given (or the current rules).
async fn render_admin(
    state: &AppState,
    s: &super::session::Session,
    notice: Option<&str>,
    draft: Option<(&str, bool)>,
    preview: bool,
    error: Option<String>,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, notice);
    let loaded = async {
        let current = rules::get(state, Some(s.account_id)).await?;
        let mut conn = state.pool.acquire().await?;
        let acceptance = server_settings::rules_acceptance(&mut conn).await?;
        Ok::<_, ApiError>((current, acceptance))
    }
    .await;
    let (current, acceptance) = match loaded {
        Ok(v) => v,
        Err(e) => return error_page(&c, e),
    };
    let (body, minor) = match draft {
        Some((body, minor)) => (body.to_owned(), minor),
        None => (
            current.as_ref().map(|r| r.body.clone()).unwrap_or_default(),
            false,
        ),
    };
    let preview_html = preview.then(|| markdown::render(&body));
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    html(
        status,
        quill_board_ui::admin_rules_page(
            &c,
            AdminRulesView {
                current,
                body,
                minor,
                preview_html,
                acceptance,
                error,
            },
        ),
    )
}

pub async fn admin_page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_admin(&state, &s, q.notice(), None, false, None).await,
        Err(r) => r,
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct RulesForm {
    csrf: Option<String>,
    body: Option<String>,
    minor: Option<String>,
    /// `save` or `preview`.
    action: Option<String>,
}

pub async fn admin_save(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<RulesForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::ServerSettings, None));
    }
    let body = form.body.clone().unwrap_or_default();
    let minor = form.minor.is_some();
    if opt(&form.action) == Some("preview") {
        return render_admin(&state, &s, None, Some((&body, minor)), true, None).await;
    }
    match rules::save(
        &state,
        s.account_id,
        &UpdateRules {
            body: body.clone(),
            minor,
        },
    )
    .await
    {
        Ok(_) => redirect(&format!("{ADMIN_RULES_PATH}?notice=rules-saved")),
        Err(e) => match form_error(&e) {
            Some(m) => render_admin(&state, &s, None, Some((&body, minor)), false, Some(m)).await,
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct AcceptanceForm {
    csrf: Option<String>,
    acceptance: Option<String>,
}

pub async fn admin_acceptance(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<AcceptanceForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Some(acceptance) = opt(&form.acceptance).and_then(|v| v.parse().ok()) else {
        return error_page(&c, ApiError::BadRequest("Choose a setting.".into()));
    };
    let body = UpdateServerSettings {
        rules_acceptance: Some(acceptance),
        ..UpdateServerSettings::default()
    };
    match server_settings::update(&state, s.account_id, &body).await {
        Ok(_) => redirect(&format!("{ADMIN_RULES_PATH}?notice=rules-acceptance-saved")),
        Err(e) => error_page(&c, e),
    }
}

/// The version a sign-up form asks to accept, when the server asks for that
/// on joining and has rules.
pub(super) async fn version_to_accept_on_join(state: &AppState) -> Result<Option<u32>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    if !server_settings::rules_acceptance(&mut conn)
        .await?
        .on_join()
    {
        return Ok(None);
    }
    Ok(rules::get(state, None).await?.map(|r| r.version))
}

/// Where the board sends a visitor whose write was refused until they
/// accept the rules.
pub(super) const READ_RULES: (&str, &str) = (RULES_PATH, "Read the server rules");
