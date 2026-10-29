//! Moderation pages for administrators: scripts, chains, common data and
//! dry runs, over the same functions as the API.

use std::collections::HashMap;

use axum::{
    Form,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
};
use quill_board_ui::{
    ModerationChainsView, ModerationDataListView, ModerationDataView, ModerationDryRunView,
    ModerationScriptView, ModerationScriptsView, Nav,
};
use quill_proto::automod::{
    DataOverride, DryRun, Hook, OnError, ReplaceScript, SaveModerationData, SetScriptMode,
    UpdateChain, UploadScript,
};
use serde::Deserialize;
use uuid::Uuid;

use super::{
    chrome, csrf_failed, error_page,
    handlers::{form_error, opt},
    html, redirect,
    session::{MaybeSession, Session},
    settings::{NoticeQuery, require_admin},
};
use crate::{
    AppState,
    error::ApiError,
    moderation::{self, api::template_list, chains, data, scripts, templates},
};

fn status_for(error: Option<&String>) -> StatusCode {
    if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    }
}

/// Checks the session and CSRF token of a form post.
#[allow(clippy::result_large_err)]
fn admin_post(
    state: &AppState,
    session: Option<Session>,
    token: Option<&str>,
) -> Result<Session, Response> {
    let s = require_admin(state, session)?;
    if !s.csrf_matches(token) {
        return Err(csrf_failed(&chrome(
            state,
            Some(&s),
            Nav::ServerSettings,
            None,
        )));
    }
    Ok(s)
}

pub async fn index(State(state): State<AppState>, MaybeSession(session): MaybeSession) -> Response {
    match require_admin(&state, session) {
        Ok(_) => redirect("/admin/moderation/scripts"),
        Err(r) => r,
    }
}

// ---- Scripts ---------------------------------------------------------------

async fn render_scripts(
    state: &AppState,
    s: &Session,
    notice: Option<&str>,
    mut view: ModerationScriptsView,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, notice);
    match scripts::list(&state.pool).await {
        Ok(list) => {
            view.scripts = list;
            view.templates = template_list();
            html(
                status_for(view.error.as_ref()),
                quill_board_ui::moderation_scripts_page(&c, view),
            )
        }
        Err(e) => error_page(&c, e),
    }
}

pub async fn scripts_page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_scripts(&state, &s, q.notice(), ModerationScriptsView::default()).await,
        Err(r) => r,
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ScriptForm {
    csrf: Option<String>,
    name: Option<String>,
    source: Option<String>,
    log_only: Option<String>,
}

pub async fn upload(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<ScriptForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let body = UploadScript {
        name: form.name.clone().unwrap_or_default(),
        source: form.source.clone().unwrap_or_default(),
        log_only: form.log_only.is_some(),
    };
    match scripts::upload(&state, s.account_id, &body).await {
        Ok(script) => redirect(&format!(
            "/admin/moderation/scripts/{}?notice=script-uploaded",
            script.id
        )),
        Err(e) => match form_error(&e) {
            Some(m) => {
                let view = ModerationScriptsView {
                    name: body.name,
                    source: body.source,
                    log_only: body.log_only,
                    error: Some(m),
                    ..ModerationScriptsView::default()
                };
                render_scripts(&state, &s, None, view).await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CsrfForm {
    csrf: Option<String>,
}

pub async fn use_template(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Form(form): Form<CsrfForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match templates::create_from(&state, s.account_id, &id, None).await {
        Ok(script) => redirect(&format!(
            "/admin/moderation/scripts/{}?notice=script-uploaded",
            script.id
        )),
        Err(e) => match form_error(&e) {
            Some(m) => {
                let view = ModerationScriptsView {
                    error: Some(m),
                    ..ModerationScriptsView::default()
                };
                render_scripts(&state, &s, None, view).await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

async fn render_script(
    state: &AppState,
    s: &Session,
    id: Uuid,
    notice: Option<&str>,
    form: Option<(String, String, String)>,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, notice);
    match scripts::get(&state.pool, id).await {
        Ok(script) => {
            let (name, source, error) = match form {
                Some((name, source, error)) => (name, source, Some(error)),
                None => (script.name.clone(), script.source.clone(), None),
            };
            let status = status_for(error.as_ref());
            html(
                status,
                quill_board_ui::moderation_script_page(
                    &c,
                    ModerationScriptView {
                        script,
                        name,
                        source,
                        error,
                    },
                ),
            )
        }
        Err(e) => error_page(&c, e),
    }
}

pub async fn script_page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<Uuid>,
    Query(q): Query<NoticeQuery>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_script(&state, &s, id, q.notice(), None).await,
        Err(r) => r,
    }
}

pub async fn script_save(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<Uuid>,
    Form(form): Form<ScriptForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let body = ReplaceScript {
        name: form.name.clone(),
        source: form.source.clone(),
    };
    match scripts::replace(&state, s.account_id, id, &body).await {
        Ok(_) => redirect(&format!(
            "/admin/moderation/scripts/{id}?notice=script-saved"
        )),
        Err(e) => match form_error(&e) {
            Some(m) => {
                let kept = (
                    form.name.unwrap_or_default(),
                    form.source.unwrap_or_default(),
                    m,
                );
                render_script(&state, &s, id, None, Some(kept)).await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ModeForm {
    csrf: Option<String>,
    mode: Option<String>,
}

pub async fn script_mode(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<Uuid>,
    Form(form): Form<ModeForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let body = match opt(&form.mode) {
        Some("on") => SetScriptMode {
            enabled: Some(true),
            log_only: Some(false),
        },
        Some("log_only") => SetScriptMode {
            enabled: Some(true),
            log_only: Some(true),
        },
        Some("off") => SetScriptMode {
            enabled: Some(false),
            log_only: None,
        },
        _ => {
            return error_page(
                &chrome(&state, Some(&s), Nav::ServerSettings, None),
                ApiError::BadRequest("Choose a setting.".into()),
            );
        }
    };
    match scripts::set_mode(&state, s.account_id, id, &body).await {
        Ok(_) => redirect(&format!(
            "/admin/moderation/scripts/{id}?notice=script-mode"
        )),
        Err(e) => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ConfirmForm {
    csrf: Option<String>,
    confirm: Option<String>,
}

pub async fn script_delete(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<Uuid>,
    Form(form): Form<ConfirmForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if form.confirm.is_none() {
        let kept = scripts::get(&state.pool, id)
            .await
            .map(|sc| (sc.name, sc.source));
        let (name, source) = kept.unwrap_or_default();
        let error = "Tick the box to confirm removing the script.".to_owned();
        return render_script(&state, &s, id, None, Some((name, source, error))).await;
    }
    match scripts::remove(&state, s.account_id, id).await {
        Ok(()) => redirect("/admin/moderation/scripts?notice=script-removed"),
        Err(e) => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
    }
}

// ---- Chains ----------------------------------------------------------------

async fn render_chains(
    state: &AppState,
    s: &Session,
    notice: Option<&str>,
    error: Option<String>,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, notice);
    let loaded = async {
        Ok::<_, ApiError>((
            chains::list(&state.pool).await?,
            scripts::list(&state.pool).await?,
        ))
    }
    .await;
    match loaded {
        Ok((chains, scripts)) => {
            let status = status_for(error.as_ref());
            html(
                status,
                quill_board_ui::moderation_chains_page(
                    &c,
                    ModerationChainsView {
                        chains,
                        scripts,
                        error,
                    },
                ),
            )
        }
        Err(e) => error_page(&c, e),
    }
}

pub async fn chains_page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_chains(&state, &s, q.notice(), None).await,
        Err(r) => r,
    }
}

/// A chain form: `on_error`, and `pos_<script id>` = its place, or blank.
pub async fn chain_save(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(hook): Path<Hook>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = match admin_post(&state, session, form.get("csrf").map(String::as_str)) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let Some(on_error) = form.get("on_error").and_then(|v| v.parse::<OnError>().ok()) else {
        return render_chains(
            &state,
            &s,
            None,
            Some("Choose what a failing script means.".into()),
        )
        .await;
    };
    let mut placed: Vec<(u32, Uuid)> = Vec::new();
    for (key, value) in &form {
        let (Some(id), value) = (key.strip_prefix("pos_"), value.trim()) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let (Ok(id), Ok(pos)) = (id.parse::<Uuid>(), value.parse::<u32>()) else {
            return render_chains(
                &state,
                &s,
                None,
                Some(format!("“{value}” isn't a position: use 1, 2, 3…")),
            )
            .await;
        };
        placed.push((pos, id));
    }
    // Ties keep a stable order: by position, then by script id.
    placed.sort();
    let body = UpdateChain {
        on_error,
        scripts: placed.into_iter().map(|(_, id)| id).collect(),
    };
    match chains::update(&state, s.account_id, hook, &body).await {
        Ok(_) => redirect("/admin/moderation/chains?notice=chain-saved"),
        Err(e) => match form_error(&e) {
            Some(m) => render_chains(&state, &s, None, Some(m)).await,
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

// ---- Common data -----------------------------------------------------------

async fn render_data_list(
    state: &AppState,
    s: &Session,
    notice: Option<&str>,
    mut view: ModerationDataListView,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, notice);
    match data::list(&state.pool).await {
        Ok(entries) => {
            view.entries = entries;
            html(
                status_for(view.error.as_ref()),
                quill_board_ui::moderation_data_list_page(&c, view),
            )
        }
        Err(e) => error_page(&c, e),
    }
}

pub async fn data_list_page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_data_list(&state, &s, q.notice(), ModerationDataListView::default()).await,
        Err(r) => r,
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct DataForm {
    csrf: Option<String>,
    name: Option<String>,
    description: Option<String>,
    content: Option<String>,
    base_version: Option<i32>,
    /// `save` or `preview`.
    action: Option<String>,
    preview_title: Option<String>,
    preview_body: Option<String>,
}

pub async fn data_create(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<DataForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let name = form.name.clone().unwrap_or_default().trim().to_owned();
    let body = SaveModerationData {
        description: form.description.clone().unwrap_or_default(),
        content: form.content.clone().unwrap_or_default(),
        base_version: None,
    };
    // Adding never overwrites: an existing name is a mistake on this form.
    if data::get(&state.pool, &name).await.is_ok() {
        let view = ModerationDataListView {
            name,
            description: body.description,
            content: body.content,
            error: Some("There's already an entry with that name. Open it to change it.".into()),
            ..ModerationDataListView::default()
        };
        return render_data_list(&state, &s, None, view).await;
    }
    match data::save(&state, s.account_id, &name, &body).await {
        Ok(_) => redirect(&format!("/admin/moderation/data/{name}?notice=data-saved")),
        Err(e) => match form_error(&e) {
            Some(m) => {
                let view = ModerationDataListView {
                    name,
                    description: body.description,
                    content: body.content,
                    error: Some(m),
                    ..ModerationDataListView::default()
                };
                render_data_list(&state, &s, None, view).await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

/// Each chain's dry-run results, for a preview of unsaved data.
type Previews = Vec<(Hook, quill_proto::automod::DryRunResult)>;

/// What the data page shows besides the saved entry.
#[derive(Default)]
struct DataExtras {
    form: Option<(String, String, i32)>,
    preview: Option<(String, String, Previews)>,
    viewing: Option<i32>,
    error: Option<String>,
}

async fn render_data(
    state: &AppState,
    s: &Session,
    name: &str,
    notice: Option<&str>,
    extras: DataExtras,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, notice);
    let loaded = async {
        let entry = data::get(&state.pool, name).await?;
        let history = data::history(&state.pool, name).await?;
        let viewing = match extras.viewing {
            Some(v) => Some(quill_proto::automod::ModerationDataContent {
                name: name.to_owned(),
                version: v,
                content: data::version(&state.pool, name, v).await?,
            }),
            None => None,
        };
        Ok::<_, ApiError>((entry, history, viewing))
    }
    .await;
    match loaded {
        Ok((entry, history, viewing)) => {
            let (description, content, base_version) = extras.form.unwrap_or_else(|| {
                (
                    entry.description.clone(),
                    entry.content.clone(),
                    entry.version,
                )
            });
            let (preview_title, preview_body, preview) = match extras.preview {
                Some((t, b, p)) => (t, b, Some(p)),
                None => (String::new(), String::new(), None),
            };
            let view = ModerationDataView {
                entry,
                history,
                description,
                content,
                base_version,
                preview_title,
                preview_body,
                preview,
                viewing,
                error: extras.error,
            };
            html(
                status_for(view.error.as_ref()),
                quill_board_ui::moderation_data_page(&c, view),
            )
        }
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct DataPageQuery {
    notice: Option<String>,
    version: Option<i32>,
}

pub async fn data_page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(name): Path<String>,
    Query(q): Query<DataPageQuery>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => {
            let extras = DataExtras {
                viewing: q.version,
                ..DataExtras::default()
            };
            render_data(&state, &s, &name, q.notice.as_deref(), extras).await
        }
        Err(r) => r,
    }
}

/// Saves an entry, or with `action=preview` runs every chain that reads it
/// with the unsaved content.
pub async fn data_save(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(name): Path<String>,
    Form(form): Form<DataForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let description = form.description.clone().unwrap_or_default();
    let content = form.content.clone().unwrap_or_default();
    let base = form.base_version.unwrap_or_default();
    let kept = Some((description.clone(), content.clone(), base));
    if opt(&form.action) == Some("preview") {
        let title = form.preview_title.clone().unwrap_or_default();
        let body = form.preview_body.clone().unwrap_or_default();
        return match preview(&state, &name, &content, &title, &body).await {
            Ok(results) => {
                let extras = DataExtras {
                    form: kept,
                    preview: Some((title, body, results)),
                    ..DataExtras::default()
                };
                render_data(&state, &s, &name, None, extras).await
            }
            Err(e) => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        };
    }
    let save = SaveModerationData {
        description,
        content,
        base_version: Some(base),
    };
    match data::save(&state, s.account_id, &name, &save).await {
        Ok(_) => redirect(&format!("/admin/moderation/data/{name}?notice=data-saved")),
        Err(e) => match form_error(&e) {
            Some(m) => {
                render_data(
                    &state,
                    &s,
                    &name,
                    None,
                    DataExtras {
                        form: kept,
                        error: Some(m),
                        ..DataExtras::default()
                    },
                )
                .await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

/// Runs each chain with a script reading `name`, on the given text, with
/// `content` in place of what's saved.
async fn preview(
    state: &AppState,
    name: &str,
    content: &str,
    title: &str,
    body: &str,
) -> Result<Previews, ApiError> {
    let entry = data::get(&state.pool, name).await?;
    let mut out = Vec::new();
    for chain in chains::list(&state.pool).await? {
        let reads = chain
            .steps
            .iter()
            .any(|st| entry.used_by.contains(&st.name));
        if !reads {
            continue;
        }
        let run = DryRun {
            hook: chain.hook,
            script: None,
            prompt: None,
            title: title.to_owned(),
            body: body.to_owned(),
            data: vec![DataOverride {
                name: name.to_owned(),
                content: content.to_owned(),
            }],
        };
        out.push((chain.hook, moderation::dry_run(state, &run).await?));
    }
    Ok(out)
}

#[derive(Debug, Default, Deserialize)]
pub struct RestoreForm {
    csrf: Option<String>,
    version: Option<i32>,
}

pub async fn data_restore(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(name): Path<String>,
    Form(form): Form<RestoreForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let Some(version) = form.version else {
        return error_page(
            &chrome(&state, Some(&s), Nav::ServerSettings, None),
            ApiError::BadRequest("Choose a version.".into()),
        );
    };
    match data::restore(&state, s.account_id, &name, version).await {
        Ok(_) => redirect(&format!(
            "/admin/moderation/data/{name}?notice=data-restored"
        )),
        Err(e) => match form_error(&e) {
            Some(m) => {
                render_data(
                    &state,
                    &s,
                    &name,
                    None,
                    DataExtras {
                        error: Some(m),
                        ..DataExtras::default()
                    },
                )
                .await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

pub async fn data_delete(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(name): Path<String>,
    Form(form): Form<ConfirmForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if form.confirm.is_none() {
        let error = Some("Tick the box to confirm deleting the entry.".to_owned());
        return render_data(
            &state,
            &s,
            &name,
            None,
            DataExtras {
                error,
                ..DataExtras::default()
            },
        )
        .await;
    }
    match data::delete(&state, s.account_id, &name).await {
        Ok(()) => redirect("/admin/moderation/data?notice=data-deleted"),
        Err(e) => match form_error(&e) {
            Some(m) => {
                render_data(
                    &state,
                    &s,
                    &name,
                    None,
                    DataExtras {
                        error: Some(m),
                        ..DataExtras::default()
                    },
                )
                .await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

// ---- Dry runs --------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct DryRunForm {
    csrf: Option<String>,
    hook: Option<String>,
    script: Option<String>,
    prompt: Option<String>,
    title: Option<String>,
    body: Option<String>,
}

async fn render_dry_run(state: &AppState, s: &Session, mut view: ModerationDryRunView) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, None);
    match scripts::list(&state.pool).await {
        Ok(list) => {
            view.scripts = list;
            html(
                status_for(view.error.as_ref()),
                quill_board_ui::moderation_dry_run_page(&c, view),
            )
        }
        Err(e) => error_page(&c, e),
    }
}

fn empty_dry_run(script: Option<Uuid>) -> ModerationDryRunView {
    ModerationDryRunView {
        scripts: Vec::new(),
        hook: Hook::PromptCreated,
        script,
        prompt: String::new(),
        title: String::new(),
        body: String::new(),
        result: None,
        error: None,
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct DryRunQuery {
    script: Option<Uuid>,
}

pub async fn dry_run_page(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<DryRunQuery>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_dry_run(&state, &s, empty_dry_run(q.script)).await,
        Err(r) => r,
    }
}

/// A prompt's id, from the id itself or a board link to it.
fn prompt_id(input: &str) -> Option<Uuid> {
    let last = input.trim().trim_end_matches('/').rsplit('/').next()?;
    last.split(['?', '#']).next()?.parse().ok()
}

pub async fn dry_run_submit(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<DryRunForm>,
) -> Response {
    let s = match admin_post(&state, session, form.csrf.as_deref()) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let mut view = empty_dry_run(opt(&form.script).and_then(|v| v.parse().ok()));
    view.hook = opt(&form.hook)
        .and_then(|v| v.parse().ok())
        .unwrap_or(Hook::PromptCreated);
    view.prompt = form.prompt.clone().unwrap_or_default();
    view.title = form.title.clone().unwrap_or_default();
    view.body = form.body.clone().unwrap_or_default();
    let prompt = match opt(&form.prompt) {
        Some(p) => match prompt_id(p) {
            Some(id) => Some(id),
            None => {
                view.error = Some("That isn't a prompt's link or id.".into());
                return render_dry_run(&state, &s, view).await;
            }
        },
        None => None,
    };
    let run = DryRun {
        hook: view.hook,
        script: view.script,
        prompt,
        title: view.title.clone(),
        body: view.body.clone(),
        data: Vec::new(),
    };
    match moderation::dry_run(&state, &run).await {
        Ok(result) => view.result = Some(result),
        Err(ApiError::NotFound) => view.error = Some("There's no such prompt or script.".into()),
        Err(e) => return error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
    }
    render_dry_run(&state, &s, view).await
}

#[cfg(test)]
mod tests {
    use super::prompt_id;

    #[test]
    fn prompt_ids_from_links() {
        let id = "0190f1d2-7c4a-7b1e-9b3a-2f6d1c8e4a10";
        assert!(prompt_id(id).is_some());
        assert!(prompt_id(&format!("https://writers.example/p/{id}?show=1")).is_some());
        assert!(prompt_id("not a prompt").is_none());
    }
}
