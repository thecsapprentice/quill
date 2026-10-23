//! Server settings pages for administrators: tag types and tags.

use axum::{
    Form,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
};
use quill_board_ui::{
    AdminTagDeleteView, AdminTagRequestsView, AdminTagTypesView, AdminTagView, AdminTagsView, Nav,
    NewTagType,
};
use quill_proto::board::{
    AdminTagQuery, ChipColor, CreateNamespace, CreateTag, TagPolicy, TagQuery, TagStatus,
    UpdateNamespace, UpdateServerSettings, UpdateTag,
};
use serde::Deserialize;

use super::{
    chrome, csrf_failed, error_page,
    handlers::{form_error, not_admin, opt},
    html, redirect,
    session::{MaybeSession, Session},
};
use crate::{
    AppState,
    error::ApiError,
    prompts, server_settings,
    tag_admin::{self, ADMIN_TAG_LIMIT},
};

/// The signed-in administrator, or the response to send instead.
// The error is an early-return response; boxing it would gain nothing.
#[allow(clippy::result_large_err)]
fn require_admin(state: &AppState, session: Option<Session>) -> Result<Session, Response> {
    match session {
        None => Err(redirect("/login")),
        Some(s) if !s.is_admin => Err(not_admin(state, &s)),
        Some(s) => Ok(s),
    }
}

fn status_for(error: Option<&String>) -> StatusCode {
    if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct NoticeQuery {
    notice: Option<String>,
}

pub async fn index(State(state): State<AppState>, MaybeSession(session): MaybeSession) -> Response {
    match require_admin(&state, session) {
        Ok(_) => redirect("/admin/settings/tag-types"),
        Err(r) => r,
    }
}

// ---- Tag types -----------------------------------------------------------

/// The chip colour fields shared by the tag type and tag forms.
#[derive(Debug, Default, Deserialize)]
pub struct ColorFields {
    /// `custom`, or anything else for no colour of its own.
    color_mode: Option<String>,
    chip_bg: Option<String>,
    /// `custom`, or anything else for automatic text.
    text_mode: Option<String>,
    chip_fg: Option<String>,
}

impl ColorFields {
    /// The chosen colours, validated; `None` when switched off.
    fn color(&self) -> Result<Option<ChipColor>, ApiError> {
        if opt(&self.color_mode) != Some("custom") {
            return Ok(None);
        }
        let text = (opt(&self.text_mode) == Some("custom"))
            .then(|| self.chip_fg.clone().unwrap_or_default());
        let color = ChipColor {
            background: self.chip_bg.clone().unwrap_or_default(),
            text,
        };
        Ok(Some(quill_proto::color::chip(&color)?))
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct TagTypeForm {
    csrf: Option<String>,
    name: Option<String>,
    display: Option<String>,
    locked: Option<String>,
    required: Option<String>,
    #[serde(flatten)]
    colors: ColorFields,
}

impl TagTypeForm {
    fn new_type(&self) -> NewTagType {
        NewTagType {
            name: self.name.clone().unwrap_or_default(),
            display: self.display.clone().unwrap_or_default(),
            locked: self.locked.is_some(),
            required: self.required.is_some(),
            color: self.colors.color().ok().flatten(),
        }
    }
}

async fn render_tag_types(
    state: &AppState,
    s: &Session,
    notice: Option<&str>,
    new: NewTagType,
    error: Option<String>,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, notice);
    let loaded = async {
        Ok::<_, ApiError>((
            prompts::namespaces(state).await?,
            server_settings::settings(state).await?.new_tags,
        ))
    }
    .await;
    match loaded {
        Ok((namespaces, new_tags)) => html(
            status_for(error.as_ref()),
            quill_board_ui::admin_tag_types_page(
                &c,
                AdminTagTypesView {
                    new_tags,
                    namespaces,
                    new,
                    error,
                },
            ),
        ),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct NewTagsForm {
    csrf: Option<String>,
    new_tags: Option<String>,
}

pub async fn new_tags_save(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<NewTagsForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Some(new_tags) = opt(&form.new_tags).and_then(|v| v.parse::<TagPolicy>().ok()) else {
        return error_page(&c, ApiError::BadRequest("Choose a setting.".into()));
    };
    let body = UpdateServerSettings {
        new_tags: Some(new_tags),
    };
    match server_settings::update(&state, s.account_id, &body).await {
        Ok(_) => redirect("/admin/settings/tag-types?notice=new-tags-saved"),
        Err(e) => error_page(&c, e),
    }
}

pub async fn tag_requests(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, opt(&q.notice));
    let loaded = async {
        let requests = tag_admin::list_tags(
            &state,
            &AdminTagQuery {
                status: Some(TagStatus::Pending),
                ..AdminTagQuery::default()
            },
        )
        .await?;
        Ok::<_, ApiError>(AdminTagRequestsView {
            requests,
            namespaces: prompts::namespaces(&state).await?,
            new_tags: server_settings::settings(&state).await?.new_tags,
        })
    }
    .await;
    match loaded {
        Ok(view) => html(
            StatusCode::OK,
            quill_board_ui::admin_tag_requests_page(&c, view),
        ),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct StatusForm {
    csrf: Option<String>,
    status: Option<String>,
    /// `requests` to return to the requests list, else the tag's page.
    back: Option<String>,
}

pub async fn tag_status(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((namespace, slug)): Path<(String, String)>,
    Form(form): Form<StatusForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, None);
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&c);
    }
    let Some(status) = opt(&form.status).and_then(|v| v.parse::<TagStatus>().ok()) else {
        return error_page(&c, ApiError::BadRequest("Choose a status.".into()));
    };
    let before = match tag_admin::get_tag(&state, &namespace, &slug).await {
        Ok(t) => t.tag.status,
        Err(e) => return error_page(&c, e),
    };
    let update = UpdateTag {
        status: Some(status),
        ..UpdateTag::default()
    };
    let notice = match (before, status) {
        (TagStatus::Pending, TagStatus::Visible) => "tag-approved",
        (_, TagStatus::Hidden) => "tag-hidden",
        _ => "tag-shown",
    };
    match tag_admin::update_tag(&state, s.account_id, &namespace, &slug, &update).await {
        Ok(_) if opt(&form.back) == Some("requests") => {
            redirect(&format!("/admin/settings/tag-requests?notice={notice}"))
        }
        Ok(tag) => redirect(&format!(
            "{}?notice={notice}",
            quill_board_ui::admin_tag_url(&tag)
        )),
        Err(e) => error_page(&c, e),
    }
}

pub async fn tag_types(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(q): Query<NoticeQuery>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_tag_types(&state, &s, opt(&q.notice), NewTagType::default(), None).await,
        Err(r) => r,
    }
}

pub async fn tag_type_create(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<TagTypeForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::ServerSettings, None));
    }
    let new = form.new_type();
    let created = async {
        let body = CreateNamespace {
            name: new.name.clone(),
            display: new.display.clone(),
            locked: new.locked,
            required: new.required,
            color: form.colors.color()?,
        };
        tag_admin::create_namespace(&state, s.account_id, &body).await
    }
    .await;
    match created {
        Ok(_) => redirect("/admin/settings/tag-types?notice=tag-type-created"),
        Err(e) => match form_error(&e) {
            Some(m) => render_tag_types(&state, &s, None, new, Some(m)).await,
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

pub async fn tag_type_update(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(name): Path<String>,
    Form(form): Form<TagTypeForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::ServerSettings, None));
    }
    // Unchecked boxes aren't sent, so absence means "off".
    let updated = async {
        let color = form.colors.color()?;
        let body = UpdateNamespace {
            display: form.display.clone(),
            locked: Some(form.locked.is_some()),
            required: Some(form.required.is_some()),
            clear_color: color.is_none(),
            color,
        };
        tag_admin::update_namespace(&state, s.account_id, &name, &body).await
    }
    .await;
    match updated {
        Ok(_) => redirect("/admin/settings/tag-types?notice=tag-type-saved"),
        Err(e) => match form_error(&e) {
            Some(m) => render_tag_types(&state, &s, None, NewTagType::default(), Some(m)).await,
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

// ---- Tags ----------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct TagsParams {
    namespace: Option<String>,
    search: Option<String>,
    notice: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct NewTagForm {
    csrf: Option<String>,
    namespace: Option<String>,
    name: Option<String>,
}

async fn render_tags(
    state: &AppState,
    s: &Session,
    params: &TagsParams,
    new: &NewTagForm,
    error: Option<String>,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, opt(&params.notice));
    let namespace = opt(&params.namespace).map(str::to_owned);
    let search = opt(&params.search).unwrap_or_default().to_owned();
    let query = AdminTagQuery {
        namespace: namespace.clone(),
        search: Some(search.clone()),
        status: None,
        limit: Some(ADMIN_TAG_LIMIT),
    };
    let result = async {
        let namespaces = prompts::namespaces(state).await?;
        let tags = tag_admin::list_tags(state, &query).await;
        // An unknown type in the URL just shows nothing.
        let tags = match tags {
            Err(ApiError::BadRequest(_)) => Vec::new(),
            other => other?,
        };
        Ok::<_, ApiError>((namespaces, tags))
    }
    .await;
    match result {
        Ok((namespaces, tags)) => {
            let truncated = tags.len() >= ADMIN_TAG_LIMIT as usize;
            let new_namespace = opt(&new.namespace)
                .or(namespace.as_deref())
                .unwrap_or_default()
                .to_owned();
            html(
                status_for(error.as_ref()),
                quill_board_ui::admin_tags_page(
                    &c,
                    AdminTagsView {
                        namespaces,
                        tags,
                        namespace,
                        search,
                        truncated,
                        new_namespace,
                        new_name: new.name.clone().unwrap_or_default(),
                        error,
                    },
                ),
            )
        }
        Err(e) => error_page(&c, e),
    }
}

pub async fn tags(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(params): Query<TagsParams>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_tags(&state, &s, &params, &NewTagForm::default(), None).await,
        Err(r) => r,
    }
}

pub async fn tag_create(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Form(form): Form<NewTagForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::ServerSettings, None));
    }
    let body = CreateTag {
        namespace: form.namespace.clone().unwrap_or_default(),
        display: form.name.clone().unwrap_or_default(),
    };
    match tag_admin::create_tag(&state, s.account_id, &body).await {
        Ok(tag) => redirect(&format!(
            "{}?notice=tag-created",
            quill_board_ui::admin_tag_url(&tag)
        )),
        Err(e) => match form_error(&e) {
            Some(m) => {
                let params = TagsParams {
                    namespace: form.namespace.clone(),
                    ..TagsParams::default()
                };
                render_tags(&state, &s, &params, &form, Some(m)).await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

async fn render_tag(
    state: &AppState,
    s: &Session,
    (namespace, slug): (&str, &str),
    notice: Option<&str>,
    form: Option<&TagForm>,
    error: Option<String>,
) -> Response {
    let c = chrome(state, Some(s), Nav::ServerSettings, notice);
    let result = async {
        let tag = tag_admin::get_tag(state, namespace, slug).await?;
        let namespace_display = prompts::namespaces(state)
            .await?
            .into_iter()
            .find(|n| n.name == tag.tag.namespace)
            .map_or_else(|| tag.tag.namespace.clone(), |n| n.display);
        let tag_options = prompts::search_tags(
            state,
            &TagQuery {
                namespace: Some(tag.tag.namespace.clone()),
                limit: Some(quill_proto::limits::MAX_PAGE_SIZE),
                ..TagQuery::default()
            },
        )
        .await?
        .into_iter()
        .filter(|t| t.key() != tag.tag.key())
        .collect();
        Ok::<_, ApiError>((tag, namespace_display, tag_options))
    }
    .await;
    match result {
        Ok((tag, namespace_display, tag_options)) => {
            let rename = form
                .and_then(|f| f.display.clone())
                .unwrap_or_else(|| tag.tag.display.clone());
            let alias_target = form.and_then(|f| f.target.clone()).unwrap_or_default();
            html(
                status_for(error.as_ref()),
                quill_board_ui::admin_tag_page(
                    &c,
                    AdminTagView {
                        tag,
                        namespace_display,
                        rename,
                        alias_target,
                        tag_options,
                        error,
                    },
                ),
            )
        }
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct TagForm {
    csrf: Option<String>,
    display: Option<String>,
    target: Option<String>,
}

pub async fn tag(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((namespace, slug)): Path<(String, String)>,
    Query(q): Query<NoticeQuery>,
) -> Response {
    match require_admin(&state, session) {
        Ok(s) => render_tag(&state, &s, (&namespace, &slug), opt(&q.notice), None, None).await,
        Err(r) => r,
    }
}

async fn change_tag(
    state: AppState,
    session: Option<Session>,
    (namespace, slug): (String, String),
    form: TagForm,
    update: UpdateTag,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::ServerSettings, None));
    }
    let notice = if update.alias_of.is_some() {
        "tag-merged"
    } else {
        "tag-saved"
    };
    match tag_admin::update_tag(&state, s.account_id, &namespace, &slug, &update).await {
        Ok(tag) => redirect(&format!(
            "{}?notice={notice}",
            quill_board_ui::admin_tag_url(&tag)
        )),
        Err(e) => match form_error(&e) {
            Some(m) => {
                render_tag(&state, &s, (&namespace, &slug), None, Some(&form), Some(m)).await
            }
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

pub async fn tag_rename(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(key): Path<(String, String)>,
    Form(form): Form<TagForm>,
) -> Response {
    let update = UpdateTag {
        display: Some(form.display.clone().unwrap_or_default()),
        ..UpdateTag::default()
    };
    change_tag(state, session, key, form, update).await
}

#[derive(Debug, Default, Deserialize)]
pub struct TagColorForm {
    csrf: Option<String>,
    #[serde(flatten)]
    colors: ColorFields,
}

pub async fn tag_color(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((namespace, slug)): Path<(String, String)>,
    Form(form): Form<TagColorForm>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !s.csrf_matches(form.csrf.as_deref()) {
        return csrf_failed(&chrome(&state, Some(&s), Nav::ServerSettings, None));
    }
    let saved = async {
        let color = form.colors.color()?;
        let update = UpdateTag {
            clear_color: color.is_none(),
            color,
            ..UpdateTag::default()
        };
        tag_admin::update_tag(&state, s.account_id, &namespace, &slug, &update).await
    }
    .await;
    match saved {
        Ok(tag) => redirect(&format!(
            "{}?notice=tag-color",
            quill_board_ui::admin_tag_url(&tag)
        )),
        Err(e) => match form_error(&e) {
            Some(m) => render_tag(&state, &s, (&namespace, &slug), None, None, Some(m)).await,
            None => error_page(&chrome(&state, Some(&s), Nav::ServerSettings, None), e),
        },
    }
}

pub async fn tag_merge(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(key): Path<(String, String)>,
    Form(form): Form<TagForm>,
) -> Response {
    let update = UpdateTag {
        alias_of: Some(form.target.clone().unwrap_or_default().trim().to_owned()),
        ..UpdateTag::default()
    };
    change_tag(state, session, key, form, update).await
}

pub async fn tag_delete_form(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((namespace, slug)): Path<(String, String)>,
) -> Response {
    let s = match require_admin(&state, session) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = chrome(&state, Some(&s), Nav::ServerSettings, None);
    let result = async {
        let tag = tag_admin::get_tag(&state, &namespace, &slug).await?;
        let ns = prompts::namespaces(&state)
            .await?
            .into_iter()
            .find(|n| n.name == tag.tag.namespace)
            .ok_or(ApiError::NotFound)?;
        Ok::<_, ApiError>((tag, ns))
    }
    .await;
    match result {
        Ok((tag, namespace)) => html(
            StatusCode::OK,
            quill_board_ui::admin_tag_delete_page(&c, AdminTagDeleteView { tag, namespace }),
        ),
        Err(e) => error_page(&c, e),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct DeleteForm {
    csrf: Option<String>,
    confirm: Option<String>,
}

pub async fn tag_delete(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path((namespace, slug)): Path<(String, String)>,
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
    if form.confirm.as_deref() != Some("yes") {
        return error_page(
            &c,
            ApiError::BadRequest("Tick the box to confirm deleting the tag.".into()),
        );
    }
    let rejecting = match tag_admin::get_tag(&state, &namespace, &slug).await {
        Ok(t) => t.tag.status == TagStatus::Pending,
        Err(e) => return error_page(&c, e),
    };
    match tag_admin::delete_tag(&state, s.account_id, &namespace, &slug).await {
        Ok(_) if rejecting => redirect("/admin/settings/tag-requests?notice=tag-rejected"),
        Ok(_) => redirect(&format!(
            "/admin/settings/tags?namespace={namespace}&notice=tag-deleted"
        )),
        Err(e) => error_page(&c, e),
    }
}
