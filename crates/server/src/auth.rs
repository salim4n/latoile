//! Bearer-token auth (D9): every route sits behind the token, preview proxy
//! included. The only open route is `/api/health`, and it answers before
//! this middleware is ever layered on.
//!
//! Two ways to present the token:
//!
//! - `Authorization: Bearer <token>` — the normal way, used by every fetch.
//! - `?token=<token>` — the documented exception: an `<iframe>` (the preview)
//!   cannot set headers, and the preview must stay behind the token (D9
//!   names it explicitly). Query tokens are accepted ONLY for paths under
//!   `/api/projects/…/preview/…`, so the exception cannot leak to the data
//!   API.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::Request;
use axum::extract::State;
use axum::middleware::Next;
use axum::response::Response;

pub async fn require_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // A plain string compare: the token is 26 random chars served over
    // localhost, so a timing oracle is not the threat model here — losing
    // the token to the network is, and TLS/host rules cover that.
    let header_ok = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {}", state.token()));

    if header_ok || query_token_ok(&request, state.token()) {
        return Ok(next.run(request).await);
    }
    Err(ApiError::unauthorized())
}

/// The iframe exception: a `?token=` query parameter, preview paths only.
fn query_token_ok(request: &Request, expected: &str) -> bool {
    if !request.uri().path().contains("/preview") {
        return false;
    }
    let Some(query) = request.uri().query() else {
        return false;
    };
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .any(|(key, value)| key == "token" && value == expected)
}
