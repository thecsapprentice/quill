//! `/admin/moderation`: scripts, chains, common data, templates and dry
//! runs over the API. Administrators only.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use quill_proto::automod::{
    DryRun, DryRunResult, Hook, ModerationChain, ModerationData, ModerationDataContent,
    ModerationDataSummary, ModerationDataVersion, ModerationScript, ReplaceScript, RestoreData,
    SaveModerationData, ScriptTemplate, SetScriptMode, UpdateChain, UploadScript, UseTemplate,
};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::{chains, data, scripts, templates};
use crate::{AppState, auth::Admin, error::ApiError, routes::ApiJson};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_scripts, upload_script))
        .routes(routes!(
            get_script,
            replace_script,
            set_script_mode,
            remove_script
        ))
        .routes(routes!(list_chains))
        .routes(routes!(update_chain))
        .routes(routes!(list_data))
        .routes(routes!(get_data, save_data, delete_data))
        .routes(routes!(data_history))
        .routes(routes!(data_version))
        .routes(routes!(restore_data))
        .routes(routes!(list_templates))
        .routes(routes!(use_template))
        .routes(routes!(dry_run))
}

#[utoipa::path(
    get, path = "/admin/moderation/scripts", tag = "admin", security(("bearer" = [])),
    summary = "Moderation scripts, by name",
    responses((status = 200, body = Vec<ModerationScript>)),
)]
async fn list_scripts(
    State(state): State<AppState>,
    _admin: Admin,
) -> Result<Json<Vec<ModerationScript>>, ApiError> {
    scripts::list(&state.pool).await.map(Json)
}

#[utoipa::path(
    post, path = "/admin/moderation/scripts", tag = "admin", security(("bearer" = [])),
    summary = "Upload a moderation script",
    description = "Refused (400) if it doesn't compile, names common data that doesn't exist, or fails on a sample post; the message names the line.",
    request_body = UploadScript,
    responses((status = 201, body = ModerationScript), (status = 400, description = "The script has a mistake"), (status = 409, description = "The name is taken")),
)]
async fn upload_script(
    State(state): State<AppState>,
    admin: Admin,
    ApiJson(body): ApiJson<UploadScript>,
) -> Result<(StatusCode, Json<ModerationScript>), ApiError> {
    let script = scripts::upload(&state, admin.0.account_id, &body).await?;
    Ok((StatusCode::CREATED, Json(script)))
}

#[utoipa::path(
    get, path = "/admin/moderation/scripts/{id}", tag = "admin", security(("bearer" = [])),
    summary = "One moderation script",
    params(("id" = Uuid, Path, description = "The script")),
    responses((status = 200, body = ModerationScript), (status = 404, description = "No such script")),
)]
async fn get_script(
    State(state): State<AppState>,
    _admin: Admin,
    Path(id): Path<Uuid>,
) -> Result<Json<ModerationScript>, ApiError> {
    scripts::get(&state.pool, id).await.map(Json)
}

#[utoipa::path(
    put, path = "/admin/moderation/scripts/{id}", tag = "admin", security(("bearer" = [])),
    summary = "Replace a script's source or name",
    description = "New source is checked as on upload, on the sample post of each hook whose chain runs it.",
    params(("id" = Uuid, Path, description = "The script")),
    request_body = ReplaceScript,
    responses((status = 200, body = ModerationScript), (status = 400, description = "The script has a mistake"), (status = 404, description = "No such script")),
)]
async fn replace_script(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<ReplaceScript>,
) -> Result<Json<ModerationScript>, ApiError> {
    scripts::replace(&state, admin.0.account_id, id, &body)
        .await
        .map(Json)
}

#[utoipa::path(
    patch, path = "/admin/moderation/scripts/{id}", tag = "admin", security(("bearer" = [])),
    summary = "Switch a script on or off, or into or out of log-only mode",
    params(("id" = Uuid, Path, description = "The script")),
    request_body = SetScriptMode,
    responses((status = 200, body = ModerationScript), (status = 404, description = "No such script")),
)]
async fn set_script_mode(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<SetScriptMode>,
) -> Result<Json<ModerationScript>, ApiError> {
    scripts::set_mode(&state, admin.0.account_id, id, &body)
        .await
        .map(Json)
}

#[utoipa::path(
    delete, path = "/admin/moderation/scripts/{id}", tag = "admin", security(("bearer" = [])),
    summary = "Remove a script, and its place in every chain",
    params(("id" = Uuid, Path, description = "The script")),
    responses((status = 204, description = "Removed"), (status = 404, description = "No such script")),
)]
async fn remove_script(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    scripts::remove(&state, admin.0.account_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get, path = "/admin/moderation/chains", tag = "admin", security(("bearer" = [])),
    summary = "Every hook's chain of scripts",
    responses((status = 200, body = Vec<ModerationChain>)),
)]
async fn list_chains(
    State(state): State<AppState>,
    _admin: Admin,
) -> Result<Json<Vec<ModerationChain>>, ApiError> {
    chains::list(&state.pool).await.map(Json)
}

#[utoipa::path(
    put, path = "/admin/moderation/chains/{hook}", tag = "admin", security(("bearer" = [])),
    summary = "Save a hook's chain: its scripts in order, and what a failing script means",
    description = "Each script newly added is first tried on the hook's sample post.",
    params(("hook" = Hook, Path, description = "The hook")),
    request_body = UpdateChain,
    responses((status = 200, body = ModerationChain), (status = 400, description = "A script fails at this hook, or doesn't exist")),
)]
async fn update_chain(
    State(state): State<AppState>,
    admin: Admin,
    Path(hook): Path<Hook>,
    ApiJson(body): ApiJson<UpdateChain>,
) -> Result<Json<ModerationChain>, ApiError> {
    chains::update(&state, admin.0.account_id, hook, &body)
        .await
        .map(Json)
}

#[utoipa::path(
    get, path = "/admin/moderation/data", tag = "admin", security(("bearer" = [])),
    summary = "Common data entries, without their content",
    responses((status = 200, body = Vec<ModerationDataSummary>)),
)]
async fn list_data(
    State(state): State<AppState>,
    _admin: Admin,
) -> Result<Json<Vec<ModerationDataSummary>>, ApiError> {
    data::list(&state.pool).await.map(Json)
}

#[utoipa::path(
    get, path = "/admin/moderation/data/{name}", tag = "admin", security(("bearer" = [])),
    summary = "One common data entry",
    params(("name" = String, Path, description = "The entry")),
    responses((status = 200, body = ModerationData), (status = 404, description = "No such entry")),
)]
async fn get_data(
    State(state): State<AppState>,
    _admin: Admin,
    Path(name): Path<String>,
) -> Result<Json<ModerationData>, ApiError> {
    data::get(&state.pool, &name).await.map(Json)
}

#[utoipa::path(
    put, path = "/admin/moderation/data/{name}", tag = "admin", security(("bearer" = [])),
    summary = "Create a common data entry, or save a new version of it",
    description = "With `baseVersion`, refused (409) if someone has saved since that version.",
    params(("name" = String, Path, description = "The entry")),
    request_body = SaveModerationData,
    responses((status = 200, body = ModerationData), (status = 400, description = "A bad name, or too large"), (status = 409, description = "Saved by someone else since")),
)]
async fn save_data(
    State(state): State<AppState>,
    admin: Admin,
    Path(name): Path<String>,
    ApiJson(body): ApiJson<SaveModerationData>,
) -> Result<Json<ModerationData>, ApiError> {
    data::save(&state, admin.0.account_id, &name, &body).await?;
    data::get(&state.pool, &name).await.map(Json)
}

#[utoipa::path(
    delete, path = "/admin/moderation/data/{name}", tag = "admin", security(("bearer" = [])),
    summary = "Delete a common data entry and its history",
    params(("name" = String, Path, description = "The entry")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "No such entry"), (status = 409, description = "A script reads it")),
)]
async fn delete_data(
    State(state): State<AppState>,
    admin: Admin,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    data::delete(&state, admin.0.account_id, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get, path = "/admin/moderation/data/{name}/versions", tag = "admin", security(("bearer" = [])),
    summary = "Every saved version of an entry, newest first",
    params(("name" = String, Path, description = "The entry")),
    responses((status = 200, body = Vec<ModerationDataVersion>), (status = 404, description = "No such entry")),
)]
async fn data_history(
    State(state): State<AppState>,
    _admin: Admin,
    Path(name): Path<String>,
) -> Result<Json<Vec<ModerationDataVersion>>, ApiError> {
    data::history(&state.pool, &name).await.map(Json)
}

#[utoipa::path(
    get, path = "/admin/moderation/data/{name}/versions/{version}", tag = "admin", security(("bearer" = [])),
    summary = "One saved version's content",
    params(("name" = String, Path, description = "The entry"), ("version" = i32, Path, description = "The version")),
    responses((status = 200, body = ModerationDataContent), (status = 404, description = "No such version")),
)]
async fn data_version(
    State(state): State<AppState>,
    _admin: Admin,
    Path((name, version)): Path<(String, i32)>,
) -> Result<Json<ModerationDataContent>, ApiError> {
    let content = data::version(&state.pool, &name, version).await?;
    Ok(Json(ModerationDataContent {
        name,
        version,
        content,
    }))
}

#[utoipa::path(
    post, path = "/admin/moderation/data/{name}/restore", tag = "admin", security(("bearer" = [])),
    summary = "Undo: save an old version's content again, as a new version",
    params(("name" = String, Path, description = "The entry")),
    request_body = RestoreData,
    responses((status = 200, body = ModerationData), (status = 404, description = "No such version")),
)]
async fn restore_data(
    State(state): State<AppState>,
    admin: Admin,
    Path(name): Path<String>,
    ApiJson(body): ApiJson<RestoreData>,
) -> Result<Json<ModerationData>, ApiError> {
    data::restore(&state, admin.0.account_id, &name, body.version).await?;
    data::get(&state.pool, &name).await.map(Json)
}

/// The templates as the API describes them.
pub fn template_list() -> Vec<ScriptTemplate> {
    templates::TEMPLATES
        .iter()
        .map(|t| ScriptTemplate {
            id: t.id.to_owned(),
            name: t.name.to_owned(),
            summary: t.summary.to_owned(),
            hooks: t.hooks.iter().filter_map(|h| h.parse().ok()).collect(),
            source: t.source.to_owned(),
            data: t.data.iter().map(|d| d.name.to_owned()).collect(),
        })
        .collect()
}

#[utoipa::path(
    get, path = "/admin/moderation/templates", tag = "admin", security(("bearer" = [])),
    summary = "The templates scripts can be started from",
    responses((status = 200, body = Vec<ScriptTemplate>)),
)]
async fn list_templates(_admin: Admin) -> Json<Vec<ScriptTemplate>> {
    Json(template_list())
}

#[utoipa::path(
    post, path = "/admin/moderation/templates/{id}", tag = "admin", security(("bearer" = [])),
    summary = "Start a new script from a template",
    description = "Creates the common data entries it reads if they don't exist yet. The copy is the administrator's from then on.",
    params(("id" = String, Path, description = "The template")),
    request_body = UseTemplate,
    responses((status = 201, body = ModerationScript), (status = 404, description = "No such template"), (status = 409, description = "The name is taken")),
)]
async fn use_template(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UseTemplate>,
) -> Result<(StatusCode, Json<ModerationScript>), ApiError> {
    let script =
        templates::create_from(&state, admin.0.account_id, &id, body.name.as_deref()).await?;
    Ok((StatusCode::CREATED, Json(script)))
}

#[utoipa::path(
    post, path = "/admin/moderation/dry-run", tag = "admin", security(("bearer" = [])),
    summary = "Run a chain or a script without doing anything",
    description = "Every script runs, whatever the one before it decided; nothing is logged or done. `data` tries unsaved common data.",
    request_body = DryRun,
    responses((status = 200, body = DryRunResult), (status = 404, description = "No such script or prompt")),
)]
async fn dry_run(
    State(state): State<AppState>,
    _admin: Admin,
    ApiJson(body): ApiJson<DryRun>,
) -> Result<Json<DryRunResult>, ApiError> {
    super::dry_run(&state, &body).await.map(Json)
}
