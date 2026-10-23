//! `/api/v1` handlers for Phase 0: instance info, accounts and personas.

use axum::{
    Json, Router,
    extract::{FromRequest, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch, post},
};
use quill_proto::{
    api::{CreatePersona, Credentials, InstanceInfo, Persona, TokenResponse, UpdatePersona},
    validate,
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{
    AppState,
    auth::{self, Authed},
    error::{ApiError, is_unique_violation},
};

/// JSON body extractor whose rejections are RFC 9457 problems.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);

pub fn api() -> Router<AppState> {
    Router::new()
        .route("/instance", get(instance))
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/personas", get(list_personas).post(create_persona))
        .route(
            "/personas/{id}",
            patch(update_persona).delete(delete_persona),
        )
        .merge(crate::prompts::router())
        .merge(crate::reports::router())
        .merge(crate::tag_admin::router())
        .merge(crate::server_settings::router())
        .merge(crate::reserved::router())
        .merge(crate::notices::router())
        .merge(crate::keys::router())
        .merge(crate::groups::router())
        .merge(crate::requests::router())
        .merge(crate::sessions::router())
        .merge(crate::events::router())
        .merge(crate::blobs::router())
}

async fn instance(State(state): State<AppState>) -> Result<Json<InstanceInfo>, ApiError> {
    let settings = crate::server_settings::settings(&state).await?;
    Ok(Json(InstanceInfo {
        domain: state.config.domain.clone(),
        software: "quill".into(),
        software_version: env!("CARGO_PKG_VERSION").into(),
        protocol_versions: quill_proto::PROTOCOL_VERSIONS
            .iter()
            .map(|v| (*v).to_owned())
            .collect(),
        registration_open: state.config.registration_open,
        email_recovery: false,
        tag_policy: settings.new_tags,
        default_maturity_max: quill_proto::board::Maturity::DEFAULT_MAX,
        limits: state.config.limits,
    }))
}

/// Sign-in attempts allowed per login name every 15 minutes.
const LOGINS_PER_NAME: usize = 10;

async fn issue_token(state: &AppState, account_id: Uuid) -> Result<TokenResponse, ApiError> {
    let token = auth::new_token()?;
    let expires_at = OffsetDateTime::now_utc() + Duration::days(state.config.token_ttl_days);
    sqlx::query("INSERT INTO api_tokens (token_hash, account_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&token.hash)
        .bind(account_id)
        .bind(expires_at)
        .execute(&state.pool)
        .await?;
    Ok(TokenResponse {
        token: token.token,
        expires_at,
    })
}

async fn register(
    State(state): State<AppState>,
    ApiJson(creds): ApiJson<Credentials>,
) -> Result<impl IntoResponse, ApiError> {
    let account_id = create_account(&state, creds).await?;
    let token = issue_token(&state, account_id).await?;
    Ok((StatusCode::CREATED, Json(token)))
}

/// Creates an account and returns its id.
pub async fn create_account(state: &AppState, creds: Credentials) -> Result<Uuid, ApiError> {
    if !state.config.registration_open {
        return Err(ApiError::Forbidden("registration is closed".into()));
    }
    let login_name = validate::username(&creds.username)?;
    validate::password(&creds.password)?;
    let password_hash = auth::hash_password(creds.password).await?;

    let account_id = Uuid::now_v7();
    let inserted =
        sqlx::query("INSERT INTO accounts (id, login_name, password_hash) VALUES ($1, $2, $3)")
            .bind(account_id)
            .bind(&login_name)
            .bind(&password_hash)
            .execute(&state.pool)
            .await;
    match inserted {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::Conflict("username unavailable".into()));
        }
        Err(e) => return Err(e.into()),
    }
    Ok(account_id)
}

async fn login(
    State(state): State<AppState>,
    ApiJson(creds): ApiJson<Credentials>,
) -> Result<Json<TokenResponse>, ApiError> {
    let account_id = check_credentials(&state, creds).await?;
    Ok(Json(issue_token(&state, account_id).await?))
}

/// Verifies a login name and password and returns the account id. Unknown
/// accounts and wrong passwords fail identically, in the same time.
pub async fn check_credentials(state: &AppState, creds: Credentials) -> Result<Uuid, ApiError> {
    // Slow down password guessing against any one login name.
    let key = format!("login:{}", creds.username.trim().to_ascii_lowercase());
    if !state.limiter.allow(
        &key,
        LOGINS_PER_NAME,
        std::time::Duration::from_secs(15 * 60),
    ) {
        return Err(ApiError::TooManyRequests);
    }
    // Opportunistic cleanup keeps the token table small without a job runner.
    sqlx::query("DELETE FROM api_tokens WHERE expires_at <= now()")
        .execute(&state.pool)
        .await?;

    let row: Option<(Uuid, String)> = match validate::username(&creds.username) {
        Ok(name) => {
            sqlx::query_as("SELECT id, password_hash FROM accounts WHERE login_name = $1")
                .bind(name)
                .fetch_optional(&state.pool)
                .await?
        }
        Err(_) => None,
    };

    // Always run one Argon2 verification so unknown accounts and wrong
    // passwords take the same time and return the same error.
    let (account_id, stored) = match row {
        Some((id, hash)) => (Some(id), hash),
        None => (None, state.dummy_hash.as_ref().clone()),
    };
    let ok = auth::verify_password(creds.password, stored).await?;
    match (account_id, ok) {
        (Some(id), true) => Ok(id),
        _ => Err(ApiError::InvalidCredentials),
    }
}

async fn logout(State(state): State<AppState>, authed: Authed) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM api_tokens WHERE token_hash = $1")
        .bind(&authed.token_hash)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

type PersonaRow = (Uuid, String, Option<String>, Option<String>, OffsetDateTime);

fn persona_from_row(
    domain: &str,
    (id, handle, display_name, summary, created_at): PersonaRow,
) -> Persona {
    Persona {
        id,
        actor_uri: quill_proto::keys::actor_uri(domain, &handle),
        handle,
        display_name,
        summary,
        created_at,
    }
}

async fn list_personas(
    State(state): State<AppState>,
    authed: Authed,
) -> Result<Json<Vec<Persona>>, ApiError> {
    personas(&state, authed.account_id).await.map(Json)
}

/// An account's personas, oldest first.
pub async fn personas(state: &AppState, account: Uuid) -> Result<Vec<Persona>, ApiError> {
    let rows: Vec<PersonaRow> = sqlx::query_as(
        "SELECT id, preferred_username, display_name, summary, created_at
         FROM actors WHERE account_id = $1 ORDER BY created_at",
    )
    .bind(account)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| persona_from_row(&state.config.domain, r))
        .collect())
}

async fn create_persona(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(body): ApiJson<CreatePersona>,
) -> Result<impl IntoResponse, ApiError> {
    let persona = add_persona(&state, authed.account_id, &body).await?;
    Ok((StatusCode::CREATED, Json(persona)))
}

/// Creates a persona for `account`.
pub async fn add_persona(
    state: &AppState,
    account: Uuid,
    body: &CreatePersona,
) -> Result<Persona, ApiError> {
    insert_persona(state, account, body, false).await
}

/// Creates a persona for the account `login`, allowing reserved handles.
/// Only the `quill-server grant-handle` command calls this; retired handles
/// stay retired even here.
pub async fn grant_persona(
    state: &AppState,
    login: &str,
    body: &CreatePersona,
) -> Result<Persona, ApiError> {
    let login = validate::username(login)?;
    let account: Uuid = sqlx::query_scalar("SELECT id FROM accounts WHERE login_name = $1")
        .bind(&login)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    insert_persona(state, account, body, true).await
}

async fn insert_persona(
    state: &AppState,
    account: Uuid,
    body: &CreatePersona,
    allow_reserved: bool,
) -> Result<Persona, ApiError> {
    let handle = validate::username(&body.handle)?;
    let display_name = validate::display_name(body.display_name.as_deref())?;
    let summary = validate::summary(body.summary.as_deref())?;
    if !allow_reserved
        && crate::reserved::reserved_by(state, &handle)
            .await?
            .is_some()
    {
        return Err(ApiError::Conflict(
            "That handle is reserved for this server's staff.".into(),
        ));
    }

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM actors WHERE account_id = $1")
        .bind(account)
        .fetch_one(&state.pool)
        .await?;
    if count >= state.config.max_personas_per_account {
        return Err(ApiError::Forbidden("persona limit reached".into()));
    }

    // A handle in use and a retired one (refused by a trigger) get the same
    // reply, so it doesn't reveal which.
    let inserted: Result<PersonaRow, sqlx::Error> = sqlx::query_as(
        "INSERT INTO actors (id, account_id, preferred_username, display_name, summary)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, preferred_username, display_name, summary, created_at",
    )
    .bind(Uuid::now_v7())
    .bind(account)
    .bind(&handle)
    .bind(&display_name)
    .bind(&summary)
    .fetch_one(&state.pool)
    .await;
    match inserted {
        Ok(row) => Ok(persona_from_row(&state.config.domain, row)),
        Err(e) if is_unique_violation(&e) => {
            Err(ApiError::Conflict("That handle isn't available.".into()))
        }
        Err(e) => Err(e.into()),
    }
}

async fn update_persona(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<UpdatePersona>,
) -> Result<Json<Persona>, ApiError> {
    // `Some("")` clears a field; `None` leaves it unchanged.
    let set_display = body.display_name.is_some();
    let display_name = validate::display_name(body.display_name.as_deref())?;
    let set_summary = body.summary.is_some();
    let summary = validate::summary(body.summary.as_deref())?;

    let row: Option<PersonaRow> = sqlx::query_as(
        "UPDATE actors SET
             display_name = CASE WHEN $3 THEN $4 ELSE display_name END,
             summary      = CASE WHEN $5 THEN $6 ELSE summary END
         WHERE id = $1 AND account_id = $2
         RETURNING id, preferred_username, display_name, summary, created_at",
    )
    .bind(id)
    .bind(authed.account_id)
    .bind(set_display)
    .bind(&display_name)
    .bind(set_summary)
    .bind(&summary)
    .fetch_optional(&state.pool)
    .await?;
    row.map(|r| Json(persona_from_row(&state.config.domain, r)))
        .ok_or(ApiError::NotFound)
}

async fn delete_persona(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    remove_persona(&state, authed.account_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes one of `account`'s personas, and with it the persona's prompts.
pub async fn remove_persona(state: &AppState, account: Uuid, id: Uuid) -> Result<(), ApiError> {
    // Phase 3 adds a federated `Delete` for the actor and its prompts here.
    let result = sqlx::query("DELETE FROM actors WHERE id = $1 AND account_id = $2")
        .bind(id)
        .bind(account)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}
