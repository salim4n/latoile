//! The HTTP edge. Every error response is `{code, message}` (contract §5);
//! internal chains go to `tracing`, never to the client.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use latoile_app::use_cases::UseCaseError;
use latoile_core::error::DomainError;
use latoile_core::ports::PortError;

/// The one error shape every route speaks.
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "a valid bearer token is required".into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    /// 409 — the resource exists but is in the wrong state for the action.
    pub(crate) fn conflict(code: &'static str, message: String) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code,
            message,
        }
    }

    pub fn not_found(what: &'static str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: format!("{what} not found"),
        }
    }

    /// The domain refused: the message is written for the user (state
    /// machines explain themselves), so it is safe to show.
    pub(crate) fn domain(e: DomainError) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "domain_refused",
            message: e.to_string(),
        }
    }

    /// An adapter or the store failed. The detail goes to the log; the
    /// client gets the shape and nothing else (guardian #6).
    pub(crate) fn internal(e: PortError) -> Self {
        tracing::warn!(error = %e, "request failed inside an adapter");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: "something went wrong".into(),
        }
    }
}

impl From<UseCaseError> for ApiError {
    fn from(e: UseCaseError) -> Self {
        match e {
            UseCaseError::Domain(d) => ApiError::domain(d),
            UseCaseError::NotFound(what) => ApiError::not_found(what),
            UseCaseError::Port(p) => ApiError::internal(p),
        }
    }
}

impl From<PortError> for ApiError {
    fn from(e: PortError) -> Self {
        ApiError::internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({"code": self.code, "message": self.message})),
        )
            .into_response()
    }
}
