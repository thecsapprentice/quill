//! Browser sessions: an HttpOnly cookie holding a random token, with a
//! per-session CSRF token for forms.

use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, HeaderValue, header, request::Parts},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use quill_proto::board::Maturity;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{AppState, auth::token_hash, error::ApiError};

/// `__Host-` cookies must be Secure, host-only and path `/`, so no subdomain
/// or other path can set or read them. Browsers treat `localhost` as secure,
/// so this also works in development.
pub const COOKIE: &str = "__Host-quill_session";

/// A signed-in browser visitor.
#[derive(Debug, Clone)]
pub struct Session {
    pub account_id: Uuid,
    pub token_hash: Vec<u8>,
    pub csrf: String,
    pub is_admin: bool,
    pub maturity_max: Maturity,
}

impl Session {
    /// Constant-time comparison of a submitted CSRF token.
    pub fn csrf_matches(&self, submitted: Option<&str>) -> bool {
        let Some(submitted) = submitted else {
            return false;
        };
        let (a, b) = (self.csrf.as_bytes(), submitted.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

fn random_token() -> Result<String, ApiError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("OS RNG failed: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.to_owned())
}

/// Starts a session and returns the `Set-Cookie` header value.
pub async fn start(state: &AppState, account_id: Uuid) -> Result<HeaderValue, ApiError> {
    let token = random_token()?;
    let ttl = Duration::days(state.config.token_ttl_days);
    sqlx::query("DELETE FROM web_sessions WHERE expires_at <= now()")
        .execute(&state.pool)
        .await?;
    sqlx::query(
        "INSERT INTO web_sessions (token_hash, account_id, csrf_token, expires_at)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(token_hash(&token))
    .bind(account_id)
    .bind(random_token()?)
    .bind(OffsetDateTime::now_utc() + ttl)
    .execute(&state.pool)
    .await?;
    let cookie = format!(
        "{COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}",
        ttl.whole_seconds()
    );
    HeaderValue::from_str(&cookie).map_err(|e| ApiError::Internal(e.into()))
}

/// Ends a session and returns a `Set-Cookie` header value that clears it.
pub async fn end(state: &AppState, session: &Session) -> Result<HeaderValue, ApiError> {
    sqlx::query("DELETE FROM web_sessions WHERE token_hash = $1")
        .bind(&session.token_hash)
        .execute(&state.pool)
        .await?;
    Ok(HeaderValue::from_static(
        "__Host-quill_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0",
    ))
}

/// The visitor's session, if they are signed in. An unknown or expired cookie
/// counts as signed out.
#[derive(Debug, Clone)]
pub struct MaybeSession(pub Option<Session>);

impl FromRequestParts<AppState> for MaybeSession {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let Some(token) = cookie_token(&parts.headers) else {
            return Ok(Self(None));
        };
        let hash = token_hash(&token);
        let row: Option<(Uuid, String, bool, String)> = sqlx::query_as(
            "SELECT s.account_id, s.csrf_token, a.is_admin, a.maturity_max::text
             FROM web_sessions s JOIN accounts a ON a.id = s.account_id
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(&hash)
        .fetch_optional(&state.pool)
        .await?;
        Ok(Self(row.map(|(account_id, csrf, is_admin, maturity)| {
            Session {
                account_id,
                token_hash: hash,
                csrf,
                is_admin,
                maturity_max: maturity.parse().unwrap_or(Maturity::DEFAULT_MAX),
            }
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_session_cookie() {
        let mut h = HeaderMap::new();
        h.append(
            header::COOKIE,
            HeaderValue::from_static("a=1; __Host-quill_session=tok; b=2"),
        );
        assert_eq!(cookie_token(&h).as_deref(), Some("tok"));
        let mut other = HeaderMap::new();
        other.append(
            header::COOKIE,
            HeaderValue::from_static("quill_session=tok"),
        );
        assert_eq!(cookie_token(&other), None);
    }

    #[test]
    fn csrf_comparison() {
        let s = Session {
            account_id: Uuid::nil(),
            token_hash: Vec::new(),
            csrf: "abc".into(),
            is_admin: false,
            maturity_max: Maturity::Teen,
        };
        assert!(s.csrf_matches(Some("abc")));
        assert!(!s.csrf_matches(Some("abd")));
        assert!(!s.csrf_matches(Some("ab")));
        assert!(!s.csrf_matches(None));
    }
}
