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
}

async fn instance(State(state): State<AppState>) -> Json<InstanceInfo> {
    Json(InstanceInfo {
        domain: state.config.domain.clone(),
        software: "quill".into(),
        software_version: env!("CARGO_PKG_VERSION").into(),
        protocol_versions: quill_proto::PROTOCOL_VERSIONS
            .iter()
            .map(|v| (*v).to_owned())
            .collect(),
        registration_open: state.config.registration_open,
        email_recovery: false,
        tag_policy: state.config.tag_policy,
        default_maturity_max: quill_proto::board::Maturity::DEFAULT_MAX,
        limits: state.config.limits,
    })
}

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
    let token = issue_token(&state, account_id).await?;
    Ok((StatusCode::CREATED, Json(token)))
}

async fn login(
    State(state): State<AppState>,
    ApiJson(creds): ApiJson<Credentials>,
) -> Result<Json<TokenResponse>, ApiError> {
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
        (Some(id), true) => Ok(Json(issue_token(&state, id).await?)),
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

fn persona_from_row((id, handle, display_name, summary, created_at): PersonaRow) -> Persona {
    Persona {
        id,
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
    let rows: Vec<PersonaRow> = sqlx::query_as(
        "SELECT id, preferred_username, display_name, summary, created_at
         FROM actors WHERE account_id = $1 ORDER BY created_at",
    )
    .bind(authed.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(persona_from_row).collect()))
}

async fn create_persona(
    State(state): State<AppState>,
    authed: Authed,
    ApiJson(body): ApiJson<CreatePersona>,
) -> Result<impl IntoResponse, ApiError> {
    let handle = validate::username(&body.handle)?;
    let display_name = validate::display_name(body.display_name.as_deref())?;
    let summary = validate::summary(body.summary.as_deref())?;

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM actors WHERE account_id = $1")
        .bind(authed.account_id)
        .fetch_one(&state.pool)
        .await?;
    if count >= state.config.max_personas_per_account {
        return Err(ApiError::Forbidden("persona limit reached".into()));
    }

    let inserted: Result<PersonaRow, sqlx::Error> = sqlx::query_as(
        "INSERT INTO actors (id, account_id, preferred_username, display_name, summary)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, preferred_username, display_name, summary, created_at",
    )
    .bind(Uuid::now_v7())
    .bind(authed.account_id)
    .bind(&handle)
    .bind(&display_name)
    .bind(&summary)
    .fetch_one(&state.pool)
    .await;
    match inserted {
        Ok(row) => Ok((StatusCode::CREATED, Json(persona_from_row(row)))),
        Err(e) if is_unique_violation(&e) => Err(ApiError::Conflict("handle unavailable".into())),
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
    row.map(|r| Json(persona_from_row(r)))
        .ok_or(ApiError::NotFound)
}

async fn delete_persona(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    // Phase 3 adds a federated `Delete` for the actor and its prompts here.
    let result = sqlx::query("DELETE FROM actors WHERE id = $1 AND account_id = $2")
        .bind(id)
        .bind(authed.account_id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
