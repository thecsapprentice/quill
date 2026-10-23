//! Board page handlers. Each GET renders a page; each POST validates the
//! form, calls the shared service functions, and redirects (or re-renders the
//! form with an error).

use std::str::FromStr;

use axum::{
    Form,
    body::Bytes,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use quill_board_ui::{
    AdminLogView, AdminReportsView, AuthMode, AuthView, BoardFilters, BoardView, Nav, PersonasView,
    PromptFormView, PromptView, ReportView, SettingsView,
};
use quill_proto::{
    api::{CreatePersona, Credentials},
    board::{
        CreatePrompt, CreateReport, Maturity, PromptQuery, PromptState, ReportReason, Resolution,
        TagQuery, UpdateAccountSettings, UpdatePrompt,
    },
    limits::MAX_PAGE_SIZE,
    validate,
};
use serde::Deserialize;
use uuid::Uuid;

use super::{
    chrome, csrf_failed, error_page, html, message,
    picker::{self, PromptPost},
    redirect,
    session::{self, MaybeSession, Session},
};
use crate::{AppState, error::ApiError, prompts, reports, routes};

// ---- Small helpers -------------------------------------------------------

/// A trimmed, non-empty form value.
pub(super) fn opt(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

fn parse_opt<T: FromStr>(value: &Option<String>) -> Option<T> {
    opt(value).and_then(|v| v.parse().ok())
}

/// Errors the visitor can fix by changing the form.
pub(super) fn form_error(err: &ApiError) -> Option<String> {
    match err {
        ApiError::BadRequest(m) | ApiError::Conflict(m) | ApiError::Forbidden(m) => Some(m.clone()),
        _ => None,
    }
}

async fn owns_actor(state: &AppState, actor: Uuid, account: Uuid) -> Result<bool, ApiError> {
    Ok(
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM actors WHERE id = $1 AND account_id = $2)",
        )
        .bind(actor)
        .bind(account)
        .fetch_one(&state.pool)
        .await?,
    )
}

async fn tag_options(state: &AppState) -> Vec<quill_proto::board::Tag> {
    prompts::search_tags(
        state,
        &TagQuery {
            limit: Some(MAX_PAGE_SIZE),
            ..TagQuery::default()
        },
    )
    .await
    .unwrap_or_default()
}

fn with_cookie(cookie: axum::http::HeaderValue, to: &str) -> Response {
    ([(header::SET_COOKIE, cookie)], Redirect::to(to)).into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct NoticeQuery {
    notice: Option<String>,
}

/// A form that only carries the CSRF token (and maybe a confirmation box).
#[derive(Debug, Default, Deserialize)]
pub struct ConfirmForm {
    csrf: Option<String>,
    confirm: Option<String>,
}

// ---- The board -----------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct BoardParams {
    include: Option<String>,
    exclude: Option<String>,
    q: Option<String>,
    maturity: Option<String>,
    state: Option<String>,
    cursor: Option<String>,
    notice: Option<String>,
}

pub async fn board(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(p): Query<BoardParams>,
) -> Response {
    let c = chrome(&state, session.as_ref(), Nav::Board, opt(&p.notice));
    let filters = BoardFilters {
        include: opt(&p.include).unwrap_or_default().to_owned(),
        exclude: opt(&p.exclude).unwrap_or_default().to_owned(),
        q: opt(&p.q).unwrap_or_default().to_owned(),
        maturity: parse_opt::<Maturity>(&p.maturity),
        state: parse_opt::<PromptState>(&p.state),
    };
    let query = PromptQuery {
        include: opt(&p.include).map(str::to_owned),
        exclude: opt(&p.exclude).map(str::to_owned),
        maturity_max: filters.maturity,
        q: opt(&p.q).map(str::to_owned),
        author: None,
        state: filters.state,
        cursor: opt(&p.cursor).map(str::to_owned),
        limit: None,
    };
    let viewer = session.as_ref().map(|s| s.account_id);
    let (items, next_cursor, error) = match prompts::search_prompts(&state, viewer, &query).await {
        Ok(page) => (page.items, page.next_cursor, None),
        Err(ApiError::BadRequest(m)) => (Vec::new(), None, Some(m)),
        Err(e) => return error_page(&c, e),
    };
    let view = BoardView {
        filters,
        prompts: items,
        next_cursor,
        cap: session
            .as_ref()
            .map_or(Maturity::DEFAULT_MAX, |s| s.maturity_max),
        namespaces: prompts::namespaces(&state).await.unwrap_or_default(),
        tag_options: tag_options(&state).await,
        error,
    };
    html(StatusCode::OK, quill_board_ui::board_page(&c, view))
}

// ---- One prompt ----------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct PromptParams {
    show: Option<String>,
    notice: Option<String>,
}

pub async fn prompt(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Query(p): Query<PromptParams>,
) -> Response {
    let c = chrome(&state, session.as_ref(), Nav::Other, opt(&p.notice));
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    let viewer = session.as_ref().map(|s| s.account_id);
    let prompt = match prompts::load_prompt(&state.pool, id, viewer, false).await {
        Ok(Some(p)) => p,
        Ok(None) => return error_page(&c, ApiError::NotFound),
        Err(e) => return error_page(&c, e),
    };
    let is_author = match viewer {
        Some(account) => match owns_actor(&state, prompt.author.id, account).await {
            Ok(v) => v,
            Err(e) => return error_page(&c, e),
        },
        None => false,
    };
    let cap = session
        .as_ref()
        .map_or(Maturity::DEFAULT_MAX, |s| s.maturity_max);
    let held_back = (!is_author && p.show.is_none() && prompt.maturity > cap).then_some(cap);
    html(
        StatusCode::OK,
        quill_board_ui::prompt_page(
            &c,
            PromptView {
                prompt,
                is_author,
                held_back,
            },
        ),
    )
}

// ---- Posting and editing -------------------------------------------------

/// What the prompt form shows besides the prompt's own fields.
struct FormTags {
    chips: Vec<quill_board_ui::TagChip>,
    add_text: String,
    errors: Vec<String>,
}

async fn prompt_form_view(
    state: &AppState,
    session: &Session,
    editing: Option<Uuid>,
    post: &PromptPost,
    tags: FormTags,
    error: Option<String>,
) -> Result<PromptFormView, ApiError> {
    Ok(PromptFormView {
        editing,
        personas: routes::personas(state, session.account_id).await?,
        persona: parse_opt(&post.persona),
        title: post.title.clone().unwrap_or_default(),
        body: post.body.clone().unwrap_or_default(),
        maturity: parse_opt(&post.maturity).unwrap_or(Maturity::General),
        chips: tags.chips,
        add_text: tags.add_text,
        tag_errors: tags.errors,
        state: parse_opt(&post.state).unwrap_or(PromptState::Open),
        error,
        tag_options: tag_options(state).await,
        tag_policy: crate::server_settings::settings(state).await?.new_tags,
        namespaces: prompts::namespaces(state).await?,
    })
}

async fn render_prompt_form(
    state: &AppState,
    session: &Session,
    editing: Option<Uuid>,
    post: &PromptPost,
    tags: FormTags,
    error: Option<String>,
) -> Response {
    let nav = if editing.is_some() {
        Nav::Other
    } else {
        Nav::Post
    };
    let c = chrome(state, Some(session), nav, None);
    let status = if error.is_some() || !tags.errors.is_empty() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    match prompt_form_view(state, session, editing, post, tags, error).await {
        Ok(view) => html(status, quill_board_ui::prompt_form_page(&c, view)),
        Err(e) => error_page(&c, e),
    }
}

/// Reads the picker. Returns the form to show again unless the visitor is
/// posting and every tag could be added; otherwise the chosen tag values.
async fn read_picker(
    state: &AppState,
    session: &Session,
    editing: Option<Uuid>,
    post: &PromptPost,
) -> Result<Vec<String>, Response> {
    let picked = match picker::preview(state, &post.tag_values()).await {
        Ok(p) => p,
        Err(e) => {
            return Err(error_page(
                &chrome(state, Some(session), Nav::Other, None),
                e,
            ));
        }
    };
    if post.saving() && picked.errors.is_empty() {
        return Ok(picked.chips.into_iter().map(|c| c.value).collect());
    }
    let tags = FormTags {
        chips: picked.chips,
        add_text: picked.unadded.join(", "),
        errors: picked.errors,
    };
    Err(render_prompt_form(state, session, editing, post, tags, None).await)
}

pub async fn new_form(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let tags = FormTags {
        chips: Vec::new(),
        add_text: String::new(),
        errors: Vec::new(),
    };
    render_prompt_form(&state, &s, None, &PromptPost::default(), tags, None).await
}

pub async fn new_submit(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    body: Bytes,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let post = PromptPost::parse(&body);
    if !s.csrf_matches(post.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::Post, None));
    }
    let tag_values = match read_picker(&state, &s, None, &post).await {
        Ok(v) => v,
        Err(form) => return form,
    };
    let again = |error: String| {
        let tag_values = tag_values.clone();
        let (state, s, post) = (&state, &s, &post);
        async move {
            let chips = picker::preview(state, &tag_values)
                .await
                .map(|p| p.chips)
                .unwrap_or_default();
            let tags = FormTags {
                chips,
                add_text: String::new(),
                errors: Vec::new(),
            };
            render_prompt_form(state, s, None, post, tags, Some(error)).await
        }
    };
    let Some(persona) = parse_opt::<Uuid>(&post.persona) else {
        return again("Choose a persona to post as.".to_owned()).await;
    };
    let Some(maturity) = parse_opt::<Maturity>(&post.maturity) else {
        return again("Choose a maturity rating.".to_owned()).await;
    };
    let create = CreatePrompt {
        persona,
        title: post.title.clone().unwrap_or_default(),
        body: post.body.clone().unwrap_or_default(),
        maturity,
        tags: tag_values.clone(),
        state: parse_opt(&post.state),
    };
    match prompts::post_prompt(&state, s.account_id, &create).await {
        Ok(p) => redirect(&format!("/p/{}?notice=posted", p.id)),
        Err(e) => match form_error(&e) {
            Some(m) => again(m).await,
            None => error_page(&chrome(&state, Some(&s), Nav::Post, None), e),
        },
    }
}

/// Loads a prompt for editing by its author.
async fn own_prompt(
    state: &AppState,
    s: &Session,
    id: &str,
) -> Result<quill_proto::board::Prompt, ApiError> {
    let id: Uuid = id.parse().map_err(|_| ApiError::NotFound)?;
    let p = prompts::load_prompt(&state.pool, id, Some(s.account_id), false)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !owns_actor(state, p.author.id, s.account_id).await? {
        return Err(ApiError::NotFound);
    }
    if p.removed {
        return Err(ApiError::Forbidden(
            "A server administrator removed this prompt, so it can't be changed.".into(),
        ));
    }
    Ok(p)
}

pub async fn edit_form(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let p = match own_prompt(&state, &s, &id).await {
        Ok(p) => p,
        Err(e) => return error_page(&chrome(&state, Some(&s), Nav::Other, None), e),
    };
    let post = PromptPost {
        persona: Some(p.author.id.to_string()),
        title: Some(p.title.clone()),
        body: Some(p.body.clone()),
        maturity: Some(p.maturity.to_string()),
        state: Some(p.state.to_string()),
        ..PromptPost::default()
    };
    let tags = FormTags {
        chips: picker::chips_for(&p.tags),
        add_text: String::new(),
        errors: Vec::new(),
    };
    render_prompt_form(&state, &s, Some(p.id), &post, tags, None).await
}

pub async fn edit_submit(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let c = chrome(&state, Some(&s), Nav::Other, None);
    let post = PromptPost::parse(&body);
    if !s.csrf_matches(post.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let p = match own_prompt(&state, &s, &id).await {
        Ok(p) => p,
        Err(e) => return error_page(&c, e),
    };
    let tag_values = match read_picker(&state, &s, Some(p.id), &post).await {
        Ok(v) => v,
        Err(form) => return form,
    };
    let update = UpdatePrompt {
        title: post.title.clone(),
        body: post.body.clone(),
        maturity: parse_opt(&post.maturity),
        tags: Some(tag_values.clone()),
        state: parse_opt(&post.state),
    };
    match prompts::edit_prompt(&state, s.account_id, p.id, &update).await {
        Ok(_) => redirect(&format!("/p/{}?notice=saved", p.id)),
        Err(e) => match form_error(&e) {
            Some(m) => {
                let chips = picker::preview(&state, &tag_values)
                    .await
                    .map(|p| p.chips)
                    .unwrap_or_default();
                let tags = FormTags {
                    chips,
                    add_text: String::new(),
                    errors: Vec::new(),
                };
                render_prompt_form(&state, &s, Some(p.id), &post, tags, Some(m)).await
            }
            None => error_page(&c, e),
        },
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct StateForm {
    csrf: Option<String>,
    state: Option<String>,
}

pub async fn set_state(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<StateForm>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let c = chrome(&state, Some(&s), Nav::Other, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    let Some(new_state) = parse_opt::<PromptState>(&form.state) else {
        return error_page(&c, ApiError::BadRequest("Choose a status.".into()));
    };
    let update = UpdatePrompt {
        state: Some(new_state),
        ..UpdatePrompt::default()
    };
    match prompts::edit_prompt(&state, s.account_id, id, &update).await {
        Ok(_) => redirect(&format!("/p/{id}?notice=status")),
        Err(e) => error_page(&c, e),
    }
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
    let c = chrome(&state, Some(&s), Nav::Other, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    if form.confirm.as_deref() != Some("yes") {
        return error_page(
            &c,
            ApiError::BadRequest("Tick the box to confirm deleting the prompt.".into()),
        );
    }
    match prompts::remove_own_prompt(&state, s.account_id, id).await {
        Ok(()) => redirect("/mine?notice=deleted"),
        Err(e) => error_page(&c, e),
    }
}

// ---- Reporting -----------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct ReportForm {
    csrf: Option<String>,
    reason: Option<String>,
    details: Option<String>,
}

async fn reportable(
    state: &AppState,
    session: Option<&Session>,
    id: &str,
) -> Result<quill_proto::board::Prompt, ApiError> {
    let id: Uuid = id.parse().map_err(|_| ApiError::NotFound)?;
    let p = prompts::load_prompt(&state.pool, id, session.map(|s| s.account_id), false)
        .await?
        .ok_or(ApiError::NotFound)?;
    if p.removed {
        return Err(ApiError::NotFound);
    }
    Ok(p)
}

pub async fn report_form(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> Response {
    let c = chrome(&state, session.as_ref(), Nav::Other, None);
    match reportable(&state, session.as_ref(), &id).await {
        Ok(prompt) => html(
            StatusCode::OK,
            quill_board_ui::report_page(
                &c,
                ReportView {
                    prompt,
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
    let c = chrome(&state, session.as_ref(), Nav::Other, None);
    if let Some(s) = &session
        && !s.csrf_matches(form.csrf.as_deref())
    {
        return csrf_failed(&c);
    }
    let prompt = match reportable(&state, session.as_ref(), &id).await {
        Ok(p) => p,
        Err(e) => return error_page(&c, e),
    };
    let reason = parse_opt::<ReportReason>(&form.reason);
    let rerender = |error: String| {
        html(
            StatusCode::BAD_REQUEST,
            quill_board_ui::report_page(
                &c,
                ReportView {
                    prompt: prompt.clone(),
                    reason,
                    details: form.details.clone().unwrap_or_default(),
                    error: Some(error),
                },
            ),
        )
    };
    let Some(reason) = reason else {
        return rerender("Choose what's wrong with the prompt.".into());
    };
    let report = CreateReport {
        prompt: prompt.id,
        reason,
        details: opt(&form.details).map(str::to_owned),
    };
    match reports::file_report(&state, session.as_ref().map(|s| s.account_id), &report).await {
        Ok(()) => redirect(&format!("/p/{}?notice=reported", prompt.id)),
        Err(ApiError::BadRequest(m)) => rerender(format!("Details {m}.")),
        Err(e) => error_page(&c, e),
    }
}

// ---- The visitor's own pages ---------------------------------------------

pub async fn mine(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let c = chrome(&state, Some(&s), Nav::Mine, opt(&q.notice));
    let query = PromptQuery {
        author: Some("me".into()),
        limit: Some(MAX_PAGE_SIZE),
        ..PromptQuery::default()
    };
    match prompts::search_prompts(&state, Some(s.account_id), &query).await {
        Ok(page) => html(
            StatusCode::OK,
            quill_board_ui::my_prompts_page(&c, page.items),
        ),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct PersonaForm {
    csrf: Option<String>,
    handle: Option<String>,
    display_name: Option<String>,
}

async fn render_personas(
    state: &AppState,
    s: &Session,
    notice: Option<&str>,
    form: &PersonaForm,
    error: Option<String>,
) -> Response {
    let c = chrome(state, Some(s), Nav::Personas, notice);
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    match routes::personas(state, s.account_id).await {
        Ok(personas) => html(
            status,
            quill_board_ui::personas_page(
                &c,
                PersonasView {
                    personas,
                    handle: form.handle.clone().unwrap_or_default(),
                    display_name: form.display_name.clone().unwrap_or_default(),
                    error,
                },
            ),
        ),
        Err(e) => error_page(&c, e),
    }
}

pub async fn personas(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    render_personas(&state, &s, opt(&q.notice), &PersonaForm::default(), None).await
}

pub async fn persona_create(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<PersonaForm>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::Personas, None));
    }
    let handle = form.handle.clone().unwrap_or_default();
    if validate::username(&handle).is_err() {
        let e = "Handles are 1-30 characters: a-z, 0-9 and underscores.".to_owned();
        return render_personas(&state, &s, None, &form, Some(e)).await;
    }
    let body = CreatePersona {
        handle,
        display_name: opt(&form.display_name).map(str::to_owned),
        summary: None,
    };
    match routes::add_persona(&state, s.account_id, &body).await {
        Ok(_) => redirect("/personas?notice=persona-created"),
        Err(ApiError::Conflict(_)) => {
            let e = "That handle is taken. Try another.".to_owned();
            render_personas(&state, &s, None, &form, Some(e)).await
        }
        Err(e) => match form_error(&e) {
            Some(m) => render_personas(&state, &s, None, &form, Some(m)).await,
            None => error_page(&chrome(&state, Some(&s), Nav::Personas, None), e),
        },
    }
}

pub async fn persona_delete(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<ConfirmForm>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let c = chrome(&state, Some(&s), Nav::Personas, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Ok(id) = id.parse::<Uuid>() else {
        return error_page(&c, ApiError::NotFound);
    };
    if form.confirm.as_deref() != Some("yes") {
        return error_page(
            &c,
            ApiError::BadRequest("Tick the box to confirm deleting the persona.".into()),
        );
    }
    match routes::remove_persona(&state, s.account_id, id).await {
        Ok(()) => redirect("/personas?notice=persona-deleted"),
        Err(e) => error_page(&c, e),
    }
}

pub async fn settings(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let c = chrome(&state, Some(&s), Nav::Settings, opt(&q.notice));
    match prompts::account_settings(&state, s.account_id).await {
        Ok(settings) => html(
            StatusCode::OK,
            quill_board_ui::settings_page(&c, SettingsView { settings }),
        ),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct SettingsForm {
    csrf: Option<String>,
    maturity_max: Option<String>,
}

pub async fn settings_save(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<SettingsForm>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    let c = chrome(&state, Some(&s), Nav::Settings, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let update = UpdateAccountSettings {
        maturity_max: parse_opt(&form.maturity_max),
    };
    match prompts::update_account_settings(&state, s.account_id, &update).await {
        Ok(_) => redirect("/settings?notice=settings"),
        Err(e) => error_page(&c, e),
    }
}

// ---- Sign in, register, sign out -----------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct AuthForm {
    username: Option<String>,
    password: Option<String>,
    confirm: Option<String>,
}

fn auth_page(
    state: &AppState,
    mode: AuthMode,
    username: &str,
    status: StatusCode,
    error: Option<String>,
) -> Response {
    let c = chrome(state, None, Nav::SignIn, None);
    html(
        status,
        quill_board_ui::auth_page(
            &c,
            AuthView {
                mode,
                username: username.to_owned(),
                error,
                registration_open: state.config.registration_open,
            },
        ),
    )
}

pub async fn login_form(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> Response {
    if session.is_some() {
        return redirect("/");
    }
    auth_page(&state, AuthMode::SignIn, "", StatusCode::OK, None)
}

pub async fn login_submit(State(state): State<AppState>, Form(form): Form<AuthForm>) -> Response {
    let username = form.username.clone().unwrap_or_default();
    let creds = Credentials {
        username: username.clone(),
        password: form.password.clone().unwrap_or_default(),
    };
    match routes::check_credentials(&state, creds).await {
        Ok(account) => match session::start(&state, account).await {
            Ok(cookie) => with_cookie(cookie, "/?notice=signed-in"),
            Err(e) => error_page(&chrome(&state, None, Nav::SignIn, None), e),
        },
        Err(ApiError::InvalidCredentials) => auth_page(
            &state,
            AuthMode::SignIn,
            &username,
            StatusCode::UNAUTHORIZED,
            Some("That login name and password don't match an account.".into()),
        ),
        Err(e) => error_page(&chrome(&state, None, Nav::SignIn, None), e),
    }
}

pub async fn register_form(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> Response {
    if session.is_some() {
        return redirect("/");
    }
    auth_page(&state, AuthMode::Register, "", StatusCode::OK, None)
}

pub async fn register_submit(
    State(state): State<AppState>,
    Form(form): Form<AuthForm>,
) -> Response {
    let username = form.username.clone().unwrap_or_default();
    let password = form.password.clone().unwrap_or_default();
    let fail = |m: &str| {
        auth_page(
            &state,
            AuthMode::Register,
            &username,
            StatusCode::BAD_REQUEST,
            Some(m.to_owned()),
        )
    };
    if validate::username(&username).is_err() {
        return fail("Login names are 1-30 characters: a-z, 0-9 and underscores.");
    }
    if validate::password(&password).is_err() {
        return fail("Passwords need at least 12 characters.");
    }
    if form.confirm.as_deref() != Some(password.as_str()) {
        return fail("The two passwords don't match.");
    }
    let creds = Credentials {
        username: username.clone(),
        password,
    };
    match routes::create_account(&state, creds).await {
        Ok(account) => match session::start(&state, account).await {
            Ok(cookie) => with_cookie(cookie, "/personas?notice=registered"),
            Err(e) => error_page(&chrome(&state, None, Nav::SignIn, None), e),
        },
        Err(ApiError::Conflict(_)) => fail("That login name is taken. Try another."),
        Err(ApiError::Forbidden(_)) => fail("This server isn't accepting new accounts right now."),
        Err(e) => error_page(&chrome(&state, None, Nav::SignIn, None), e),
    }
}

pub async fn logout(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<ConfirmForm>,
) -> Response {
    let Some(s) = session else {
        return redirect("/");
    };
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::Other, None));
    }
    match session::end(&state, &s).await {
        Ok(cookie) => with_cookie(cookie, "/?notice=signed-out"),
        Err(e) => error_page(&chrome(&state, Some(&s), Nav::Other, None), e),
    }
}

// ---- Administration ------------------------------------------------------

pub(super) fn not_admin(state: &AppState, s: &Session) -> Response {
    message(
        &chrome(state, Some(s), Nav::Admin, None),
        StatusCode::FORBIDDEN,
        "Administrators only",
        "Only this server's administrators can see this page.",
    )
}

#[derive(Debug, Default, Deserialize)]
pub struct AdminReportsParams {
    all: Option<String>,
    notice: Option<String>,
}

pub async fn admin_reports(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(p): Query<AdminReportsParams>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    if !s.is_admin {
        return not_admin(&state, &s);
    }
    let c = chrome(&state, Some(&s), Nav::Admin, opt(&p.notice));
    let all = p.all.is_some();
    match reports::reports(&state, all).await {
        Ok(reports) => html(
            StatusCode::OK,
            quill_board_ui::admin_reports_page(&c, AdminReportsView { reports, all }),
        ),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ResolveForm {
    csrf: Option<String>,
    action: Option<String>,
}

pub async fn admin_resolve(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<ResolveForm>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    if !s.is_admin {
        return not_admin(&state, &s);
    }
    let c = chrome(&state, Some(&s), Nav::Admin, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let (Ok(id), Some(action)) = (id.parse::<Uuid>(), parse_opt::<Resolution>(&form.action)) else {
        return error_page(&c, ApiError::NotFound);
    };
    match reports::resolve(&state, s.account_id, id, action).await {
        Ok(_) | Err(ApiError::Conflict(_)) => redirect("/admin/reports?notice=resolved"),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct LogParams {
    before: Option<i64>,
}

pub async fn admin_log(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(p): Query<LogParams>,
) -> Response {
    let Some(s) = session else {
        return redirect("/login");
    };
    if !s.is_admin {
        return not_admin(&state, &s);
    }
    let c = chrome(&state, Some(&s), Nav::Admin, None);
    const PAGE: u32 = 50;
    match reports::admin_log(&state, p.before, Some(PAGE)).await {
        Ok(entries) => {
            let older = (entries.len() == PAGE as usize)
                .then(|| {
                    entries
                        .last()
                        .map(|e| format!("/admin/log?before={}", e.id))
                })
                .flatten();
            html(
                StatusCode::OK,
                quill_board_ui::admin_log_page(&c, AdminLogView { entries, older }),
            )
        }
        Err(e) => error_page(&c, e),
    }
}
