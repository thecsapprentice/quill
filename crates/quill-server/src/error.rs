//! API errors, rendered as RFC 9457 problem details.

use axum::{
    extract::rejection::JsonRejection,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use quill_proto::{
    api::{PROBLEM_CONTENT_TYPE, Problem},
    codes::{self, ErrorCode},
};

/// A refusal or failure. Every variant with a message also carries its
/// error code (#76), from `quill_proto::codes`; the rest have fixed codes.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{1}")]
    BadRequest(ErrorCode, String),
    #[error("authentication required")]
    Unauthorized,
    /// Deliberately identical for unknown accounts and wrong passwords.
    #[error("invalid username or password")]
    InvalidCredentials,
    #[error("{1}")]
    Forbidden(ErrorCode, String),
    #[error("not found")]
    NotFound,
    #[error("{1}")]
    Conflict(ErrorCode, String),
    /// What was asked for has been deleted for good (2k: a log read from
    /// before its deleted part).
    #[error("{1}")]
    Gone(ErrorCode, String),
    #[error("request body too large")]
    PayloadTooLarge,
    #[error("unsupported media type")]
    UnsupportedMediaType,
    /// A moderation script refused the submission; the message is the
    /// administrators' wording, for the writer.
    #[error("{1}")]
    ContentRejected(ErrorCode, String),
    /// The account must accept the server rules before writing (#42).
    #[error("{1}")]
    RulesNotAccepted(ErrorCode, String),
    #[error("too many requests; try again later")]
    TooManyRequests,
    #[error("internal error")]
    Internal(#[source] anyhow::Error),
}

impl ApiError {
    /// The error code sent as the problem's `code` member.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::BadRequest(code, _)
            | Self::Forbidden(code, _)
            | Self::Conflict(code, _)
            | Self::Gone(code, _)
            | Self::ContentRejected(code, _)
            | Self::RulesNotAccepted(code, _) => *code,
            Self::Unauthorized => codes::SIGNED_OUT,
            Self::InvalidCredentials => codes::WRONG_LOGIN,
            Self::NotFound => codes::NOT_FOUND,
            Self::PayloadTooLarge => codes::TOO_LARGE,
            Self::UnsupportedMediaType => codes::WRONG_MEDIA_TYPE,
            Self::TooManyRequests => codes::TOO_MANY,
            Self::Internal(_) => codes::SERVER_FAULT,
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(..) => StatusCode::BAD_REQUEST,
            Self::Unauthorized | Self::InvalidCredentials => StatusCode::UNAUTHORIZED,
            Self::Forbidden(..) => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(..) => StatusCode::CONFLICT,
            Self::Gone(..) => StatusCode::GONE,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::ContentRejected(..) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RulesNotAccepted(..) => StatusCode::FORBIDDEN,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn slug(&self) -> &'static str {
        match self {
            Self::BadRequest(..) => "bad-request",
            Self::Unauthorized => "unauthorized",
            Self::InvalidCredentials => "invalid-credentials",
            Self::Forbidden(..) => "forbidden",
            Self::NotFound => "not-found",
            Self::Conflict(..) => "conflict",
            Self::Gone(..) => "gone",
            Self::PayloadTooLarge => "payload-too-large",
            Self::UnsupportedMediaType => "unsupported-media-type",
            Self::ContentRejected(..) => "content-rejected",
            Self::RulesNotAccepted(..) => "rules-not-accepted",
            Self::TooManyRequests => "too-many-requests",
            Self::Internal(_) => "internal",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Self::Internal(err) = &self {
            // Log the cause server-side only; never return it to the client.
            tracing::error!(error = ?err, "internal error");
        }
        let status = self.status();
        let problem = Problem {
            kind: format!("https://quill.example/problems/{}", self.slug()),
            title: status.canonical_reason().unwrap_or("error").to_owned(),
            status: status.as_u16(),
            detail: match &self {
                Self::Internal(_) => None,
                other => Some(other.to_string()),
            },
            code: Some(self.code().0),
        };
        let body = serde_json::to_vec(&problem).unwrap_or_default();
        (status, [(header::CONTENT_TYPE, PROBLEM_CONTENT_TYPE)], body).into_response()
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        match rejection.status() {
            StatusCode::PAYLOAD_TOO_LARGE => Self::PayloadTooLarge,
            StatusCode::UNSUPPORTED_MEDIA_TYPE => Self::UnsupportedMediaType,
            _ => Self::BadRequest(codes::BAD_BODY, rejection.body_text()),
        }
    }
}

impl From<quill_proto::validate::ValidationError> for ApiError {
    fn from(err: quill_proto::validate::ValidationError) -> Self {
        Self::BadRequest(err.code(), err.to_string())
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(err.into())
    }
}

/// True if `err` is a Postgres unique-constraint violation.
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .is_some_and(|db| db.is_unique_violation())
}
