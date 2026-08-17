//! `/api/agent-auth` — click-to-login for Claude and Codex. Behind the D9
//! token like everything else; errors in the contract's `{code, message}`.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use latoile_agents::{AuthError, AuthProvider, AuthSessionView};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct AuthSessionDto {
    pub session_id: String,
    pub provider: &'static str,
    pub status: &'static str,
    pub url: Option<String>,
    pub input_required: bool,
    pub user_code: Option<String>,
    pub hint: Option<String>,
    pub error: Option<String>,
}

impl From<&AuthSessionView> for AuthSessionDto {
    fn from(s: &AuthSessionView) -> Self {
        Self {
            session_id: s.id.clone(),
            provider: s.provider.as_str(),
            status: s.status.as_str(),
            url: s.url.clone(),
            input_required: s.input_required,
            user_code: s.user_code.clone(),
            hint: s.hint.clone(),
            error: s.error.clone(),
        }
    }
}

fn map_error(e: AuthError) -> ApiError {
    match e {
        AuthError::Unknown => ApiError::not_found("auth session"),
        // 409: the session exists but is in the wrong state for a code.
        AuthError::NotWaiting => ApiError::conflict("not_waiting", e.to_string()),
        // Codex confirms itself on the site — there is nowhere to paste.
        AuthError::InputNotRequired => ApiError::conflict("input_not_required", e.to_string()),
        AuthError::Spawn(_) => ApiError::internal(latoile_core::ports::PortError(e.to_string())),
    }
}

#[derive(Deserialize)]
pub struct StartBody {
    /// "claude" (default) or "codex".
    provider: Option<String>,
}

pub async fn start(
    State(state): State<AppState>,
    body: Option<Json<StartBody>>,
) -> Result<Json<AuthSessionDto>, ApiError> {
    let name = body
        .and_then(|Json(b)| b.provider)
        .unwrap_or_else(|| "claude".into());
    let provider = AuthProvider::parse(&name)
        .ok_or_else(|| ApiError::bad_request(format!("unknown provider {name:?}")))?;
    let session = state.agent_auth.start(provider).await.map_err(map_error)?;
    Ok(Json(AuthSessionDto::from(&session)))
}

pub async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AuthSessionDto>, ApiError> {
    let session = state
        .agent_auth
        .status(&id)
        .ok_or(ApiError::not_found("auth session"))?;
    Ok(Json(AuthSessionDto::from(&session)))
}

/// `GET /api/agent-auth/status` — both providers, per their own CLIs.
pub async fn status_all(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (claude, codex) = tokio::join!(
        state.agent_auth.provider_status(AuthProvider::Claude),
        state.agent_auth.provider_status(AuthProvider::Codex)
    );
    Json(serde_json::json!({
        "claude": {"authenticated": claude.authenticated, "detail": claude.detail},
        "codex": {"authenticated": codex.authenticated, "detail": codex.detail},
    }))
}

#[derive(Deserialize)]
pub struct DisconnectBody {
    provider: String,
}

/// `POST /api/agent-auth/disconnect` — the provider's own logout, then the
/// status as it stands after.
pub async fn disconnect(
    State(state): State<AppState>,
    Json(body): Json<DisconnectBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let provider = AuthProvider::parse(&body.provider)
        .ok_or_else(|| ApiError::bad_request(format!("unknown provider {:?}", body.provider)))?;
    let status = state.agent_auth.disconnect(provider).await;
    Ok(Json(serde_json::json!({
        "authenticated": status.authenticated,
        "detail": status.detail,
    })))
}

#[derive(Deserialize)]
pub struct CodeBody {
    code: String,
}

pub async fn submit_code(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CodeBody>,
) -> Result<Json<AuthSessionDto>, ApiError> {
    let session = state
        .agent_auth
        .submit_code(&id, &body.code)
        .await
        .map_err(map_error)?;
    Ok(Json(AuthSessionDto::from(&session)))
}
