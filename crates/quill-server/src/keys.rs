//! The device directory: each persona's identity key and certified devices.
//!
//! Clients generate every key; the server stores and publishes public halves
//! only. It still checks certificates and revocations with `quill-certs`
//! (public-key operations, which the dependency ban allows) so it never
//! publishes anything that wouldn't verify. Clients check again for
//! themselves.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post, put},
};
use quill_certs::{CertError, DeviceCertificate, DeviceRevocation, Expected};
use quill_proto::keys::{
    DeviceInfo, PersonaKeys, RegisterDevice, RevokeDevice, SetIdentityKey, actor_uri, b64, unb64,
};
use uuid::Uuid;

use crate::{
    AppState,
    auth::Authed,
    error::{ApiError, is_unique_violation},
    routes::ApiJson,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/personas/{id}/identity-key", put(set_identity_key))
        .route(
            "/personas/{id}/devices",
            get(own_keys).post(register_device),
        )
        .route(
            "/personas/{id}/devices/{device}/revoke",
            post(revoke_device),
        )
        .route("/actors/{handle}/keys", get(public_keys))
}

/// A persona and its current identity key.
struct Persona {
    id: Uuid,
    handle: String,
    actor_uri: String,
    identity: Option<(Vec<u8>, u32)>,
}

type PersonaRow = (Uuid, String, Option<Vec<u8>>, Option<i32>);

fn persona(state: &AppState, (id, handle, key, version): PersonaRow) -> Persona {
    Persona {
        id,
        actor_uri: actor_uri(&state.config.domain, &handle),
        handle,
        identity: key.zip(version.and_then(|v| u32::try_from(v).ok())),
    }
}

/// One of the caller's own personas.
async fn owned(state: &AppState, account: Uuid, id: Uuid) -> Result<Persona, ApiError> {
    let row: Option<PersonaRow> = sqlx::query_as(
        "SELECT id, preferred_username, identity_key, identity_key_version
         FROM actors WHERE id = $1 AND account_id = $2",
    )
    .bind(id)
    .bind(account)
    .fetch_optional(&state.pool)
    .await?;
    row.map(|r| persona(state, r)).ok_or(ApiError::NotFound)
}

fn bytes(field: &str, text: &str) -> Result<Vec<u8>, ApiError> {
    unb64(text).ok_or_else(|| ApiError::BadRequest(format!("{field} is not base64url")))
}

fn rejected(what: &str, e: &CertError) -> ApiError {
    ApiError::BadRequest(format!("{what} refused: {e}"))
}

fn now_ms() -> u64 {
    u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000).unwrap_or(0)
}

type DeviceRow = (Uuid, Vec<u8>, i32, bool, Vec<u8>, Option<Vec<u8>>);

fn device_info((id, key, version, limited, cert, revocation): DeviceRow) -> DeviceInfo {
    DeviceInfo {
        id,
        device_key: b64(&key),
        key_version: u32::try_from(version).unwrap_or(0),
        limited,
        certificate: b64(&cert),
        revocation: revocation.as_deref().map(b64),
    }
}

/// The columns `DeviceRow` reads, as a literal for `concat!`.
macro_rules! device_columns {
    () => {
        "id, device_key, key_version, limited, certificate, revocation"
    };
}

/// Everything published about a persona's keys.
async fn keys_of(state: &AppState, p: &Persona) -> Result<PersonaKeys, ApiError> {
    // Ordered by the random id: no hint of which came first.
    let rows: Vec<DeviceRow> = sqlx::query_as(concat!(
        "SELECT ",
        device_columns!(),
        " FROM devices WHERE actor_id = $1 ORDER BY id"
    ))
    .bind(p.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(PersonaKeys {
        actor_uri: p.actor_uri.clone(),
        handle: p.handle.clone(),
        identity_key: p.identity.as_ref().map(|(k, _)| b64(k)),
        key_version: p.identity.as_ref().map(|(_, v)| *v),
        devices: rows.into_iter().map(device_info).collect(),
    })
}

/// `PUT /personas/{id}/identity-key`: sets the first key, or replaces it
/// with a higher version. Repeating the current key is accepted.
async fn set_identity_key(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<SetIdentityKey>,
) -> Result<Json<PersonaKeys>, ApiError> {
    let p = owned(&state, authed.account_id, id).await?;
    let key = bytes("publicKey", &body.public_key)?;
    quill_certs::public_key(&key).map_err(|e| rejected("identity key", &e))?;
    if body.key_version == 0 {
        return Err(ApiError::BadRequest("keyVersion starts at 1".into()));
    }
    if let Some((current, version)) = &p.identity {
        if *current == key && *version == body.key_version {
            return keys_of(&state, &p).await.map(Json);
        }
        if body.key_version <= *version {
            return Err(ApiError::Conflict(format!(
                "This persona already has identity key version {version}; a replacement needs a higher version."
            )));
        }
    }
    let version = i32::try_from(body.key_version)
        .map_err(|_| ApiError::BadRequest("keyVersion is too large".into()))?;
    let updated =
        sqlx::query("UPDATE actors SET identity_key = $2, identity_key_version = $3 WHERE id = $1")
            .bind(p.id)
            .bind(&key)
            .bind(version)
            .execute(&state.pool)
            .await;
    match updated {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::Conflict(
                "That identity key is already in use.".into(),
            ));
        }
        Err(e) => return Err(e.into()),
    }
    let p = owned(&state, authed.account_id, id).await?;
    keys_of(&state, &p).await.map(Json)
}

async fn own_keys(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
) -> Result<Json<PersonaKeys>, ApiError> {
    let p = owned(&state, authed.account_id, id).await?;
    keys_of(&state, &p).await.map(Json)
}

/// `POST /personas/{id}/devices`: registers a device whose certificate
/// verifies under the persona's current identity key, or certifies an
/// existing device again after the identity key was replaced.
async fn register_device(
    State(state): State<AppState>,
    authed: Authed,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<RegisterDevice>,
) -> Result<Json<DeviceInfo>, ApiError> {
    let p = owned(&state, authed.account_id, id).await?;
    let Some((identity, version)) = &p.identity else {
        return Err(ApiError::Conflict(
            "Set this persona's identity key first.".into(),
        ));
    };
    let raw = bytes("certificate", &body.certificate)?;
    let cert = DeviceCertificate::verify(
        &raw,
        &Expected {
            actor_uri: &p.actor_uri,
            identity_key: identity,
            identity_key_version: *version,
            device_key: None,
            now_ms: now_ms(),
        },
    )
    .map_err(|e| rejected("certificate", &e))?;
    let key_version = i32::try_from(cert.identity_key_version).unwrap_or(i32::MAX);

    let mut tx = state.pool.begin().await?;
    let existing: Option<(Uuid, Uuid, i32, Vec<u8>, bool)> = sqlx::query_as(
        "SELECT id, actor_id, key_version, certificate, revocation IS NOT NULL
         FROM devices WHERE device_key = $1 FOR UPDATE",
    )
    .bind(cert.device_key.as_slice())
    .fetch_optional(&mut *tx)
    .await?;
    let device_id = match existing {
        Some((_, actor, ..)) if actor != p.id => {
            return Err(ApiError::Conflict(
                "That device key is already registered.".into(),
            ));
        }
        Some((_, _, _, _, true)) => {
            return Err(ApiError::Conflict(
                "That device was revoked; revocation is permanent. Make a new device key.".into(),
            ));
        }
        Some((device, _, _, stored, _)) if stored == raw => device,
        Some((device, _, old, ..)) if old < key_version => {
            sqlx::query(
                "UPDATE devices SET certificate = $2, key_version = $3, limited = $4 WHERE id = $1",
            )
            .bind(device)
            .bind(&raw)
            .bind(key_version)
            .bind(cert.limited)
            .execute(&mut *tx)
            .await?;
            device
        }
        Some(_) => {
            return Err(ApiError::Conflict(
                "That device is already certified.".into(),
            ));
        }
        None => {
            let device = Uuid::new_v4();
            let inserted = sqlx::query(
                "INSERT INTO devices (id, actor_id, device_key, key_version, limited, certificate)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(device)
            .bind(p.id)
            .bind(cert.device_key.as_slice())
            .bind(key_version)
            .bind(cert.limited)
            .bind(&raw)
            .execute(&mut *tx)
            .await;
            match inserted {
                Ok(_) => device,
                Err(e) if is_unique_violation(&e) => {
                    return Err(ApiError::Conflict(
                        "That device key is already registered.".into(),
                    ));
                }
                Err(e) => return Err(e.into()),
            }
        }
    };
    let row: DeviceRow = sqlx::query_as(concat!(
        "SELECT ",
        device_columns!(),
        " FROM devices WHERE id = $1"
    ))
    .bind(device_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(device_info(row)))
}

/// `POST /personas/{id}/devices/{device}/revoke`: records a revocation
/// signed by the persona's identity key. Permanent.
async fn revoke_device(
    State(state): State<AppState>,
    authed: Authed,
    Path((id, device)): Path<(Uuid, Uuid)>,
    ApiJson(body): ApiJson<RevokeDevice>,
) -> Result<Json<DeviceInfo>, ApiError> {
    let p = owned(&state, authed.account_id, id).await?;
    let Some((identity, _)) = &p.identity else {
        return Err(ApiError::NotFound);
    };
    let raw = bytes("revocation", &body.revocation)?;
    let revocation = DeviceRevocation::verify(&raw, &p.actor_uri, identity)
        .map_err(|e| rejected("revocation", &e))?;

    let mut tx = state.pool.begin().await?;
    let row: Option<(Vec<u8>, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT device_key, revocation FROM devices WHERE id = $1 AND actor_id = $2 FOR UPDATE",
    )
    .bind(device)
    .bind(p.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((key, existing)) = row else {
        return Err(ApiError::NotFound);
    };
    if key != revocation.device_key {
        return Err(ApiError::BadRequest(
            "The revocation is for a different device.".into(),
        ));
    }
    match existing {
        Some(stored) if stored == raw => {}
        Some(_) => {
            return Err(ApiError::Conflict("That device is already revoked.".into()));
        }
        None => {
            sqlx::query("UPDATE devices SET revocation = $2 WHERE id = $1")
                .bind(device)
                .bind(&raw)
                .execute(&mut *tx)
                .await?;
        }
    }
    let row: DeviceRow = sqlx::query_as(concat!(
        "SELECT ",
        device_columns!(),
        " FROM devices WHERE id = $1"
    ))
    .bind(device)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(device_info(row)))
}

/// `GET /actors/{handle}/keys`: public, for other writers' clients.
async fn public_keys(
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Json<PersonaKeys>, ApiError> {
    let row: Option<PersonaRow> = sqlx::query_as(
        "SELECT id, preferred_username, identity_key, identity_key_version
         FROM actors WHERE preferred_username = lower($1)",
    )
    .bind(&handle)
    .fetch_optional(&state.pool)
    .await?;
    let p = row.map(|r| persona(&state, r)).ok_or(ApiError::NotFound)?;
    keys_of(&state, &p).await.map(Json)
}
