//! Password hashing, API tokens and the authenticated-account extractor.
//!
//! These protect *login* only. Nothing here touches session keys, which never
//! reach the server.

use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use axum::{extract::FromRequestParts, http::request::Parts};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{AppState, error::ApiError};

/// Prefix that makes leaked tokens easy for secret scanners to spot.
const TOKEN_PREFIX: &str = "qt_";

/// Argon2id with the OWASP-recommended baseline (19 MiB, 2 passes, 1 lane).
fn argon2() -> Argon2<'static> {
    let params = Params::new(19 * 1024, 2, 1, None).unwrap_or_default();
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn random_bytes<const N: usize>() -> anyhow::Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| anyhow::anyhow!("OS RNG failed: {e}"))?;
    Ok(buf)
}

/// Hashes a password on a blocking thread. Returns a PHC string.
pub async fn hash_password(password: String) -> Result<String, ApiError> {
    let password = Zeroizing::new(password);
    tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        // argon2 draws a random salt from the OS RNG.
        let hash = argon2()
            .hash_password(password.as_bytes())
            .map_err(|e| anyhow::anyhow!("argon2: {e}"))?;
        Ok(hash.to_string())
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .map_err(ApiError::Internal)
}

/// Verifies a password on a blocking thread. Malformed stored hashes count as
/// a mismatch.
pub async fn verify_password(password: String, stored: String) -> Result<bool, ApiError> {
    let password = Zeroizing::new(password);
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&stored)
            .map(|parsed| {
                argon2()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))
}

/// A freshly minted API token: the secret for the client, the hash for the
/// database.
pub struct NewToken {
    pub token: String,
    pub hash: Vec<u8>,
}

impl std::fmt::Debug for NewToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NewToken(<redacted>)")
    }
}

pub fn new_token() -> Result<NewToken, ApiError> {
    let bytes = random_bytes::<32>().map_err(ApiError::Internal)?;
    let token = format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes));
    let hash = token_hash(&token);
    Ok(NewToken { token, hash })
}

pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// The account behind a valid bearer token.
#[derive(Debug, Clone)]
pub struct Authed {
    pub account_id: Uuid,
    pub token_hash: Vec<u8>,
}

impl FromRequestParts<AppState> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(ApiError::Unauthorized)?;
        let token = header
            .strip_prefix("Bearer ")
            .filter(|t| t.starts_with(TOKEN_PREFIX))
            .ok_or(ApiError::Unauthorized)?;
        let hash = token_hash(token);
        let account_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT account_id FROM api_tokens WHERE token_hash = $1 AND expires_at > now()",
        )
        .bind(&hash)
        .fetch_optional(&state.pool)
        .await?;
        account_id
            .map(|account_id| Authed {
                account_id,
                token_hash: hash,
            })
            .ok_or(ApiError::Unauthorized)
    }
}

/// An optional sign-in: anonymous when no `Authorization` header is sent.
/// A header that is present but invalid is still rejected, so a client with
/// an expired token learns it rather than silently browsing anonymously.
#[derive(Debug, Clone)]
pub struct MaybeAuthed(pub Option<Authed>);

impl FromRequestParts<AppState> for MaybeAuthed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if parts
            .headers
            .contains_key(axum::http::header::AUTHORIZATION)
        {
            Ok(Self(Some(Authed::from_request_parts(parts, state).await?)))
        } else {
            Ok(Self(None))
        }
    }
}

/// A signed-in server administrator.
#[derive(Debug, Clone)]
pub struct Admin(pub Authed);

impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let authed = Authed::from_request_parts(parts, state).await?;
        let is_admin: bool = sqlx::query_scalar("SELECT is_admin FROM accounts WHERE id = $1")
            .bind(authed.account_id)
            .fetch_optional(&state.pool)
            .await?
            .unwrap_or(false);
        if is_admin {
            Ok(Self(authed))
        } else {
            Err(ApiError::Forbidden("administrators only".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn password_round_trip() {
        let hash = hash_password("correct horse battery".into()).await.unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(
            verify_password("correct horse battery".into(), hash.clone())
                .await
                .unwrap()
        );
        assert!(
            !verify_password("wrong horse battery".into(), hash)
                .await
                .unwrap()
        );
        assert!(
            !verify_password("x".into(), "not a hash".into())
                .await
                .unwrap()
        );
    }

    #[test]
    fn tokens_are_unique_and_hashed() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_ne!(a.token, b.token);
        assert!(a.token.starts_with(TOKEN_PREFIX));
        assert_eq!(a.hash, token_hash(&a.token));
        assert_eq!(a.hash.len(), 32);
        assert!(!format!("{a:?}").contains(&a.token));
    }
}
