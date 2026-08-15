//! Bearer-token auth (D9): every route sits behind the token, preview proxy
//! included. The only open route is `/api/health`, and it answers before
//! this middleware is ever layered on.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::State;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

pub async fn require_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    // A plain string compare: the token is 26 random chars served over
    // localhost, so a timing oracle is not the threat model here — losing
    // the token to the network is, and TLS/host rules cover that.
    match header {
        Some(value) if value == format!("Bearer {}", state.token()) => {
            Ok(next.run(request).await)
        }
        _ => Err(ApiError::unauthorized()),
    }
}
