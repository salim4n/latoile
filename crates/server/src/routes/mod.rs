//! The router. Every handler extracts, validates, delegates — no decisions
//! live here (contract §2, guardian #5). `/api/health` is the only open
//! route; everything else sits behind the bearer token (D9).

mod agent_auth;
mod approvals;
mod architecture;
pub mod dto;
mod events;
mod github;
mod messages;
mod preview;
mod projects;
mod runs;
mod settings;
mod specs;
mod tasks;

use crate::auth;
use crate::state::AppState;
use axum::http::StatusCode;
use axum::routing::{any, get, patch, post};
use axum::{middleware, Json, Router};

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/projects", get(projects::list).post(projects::create))
        .route("/api/projects/{id}", get(projects::get))
        .route(
            "/api/projects/{id}/architecture",
            get(architecture::get).delete(architecture::cancel),
        )
        .route(
            "/api/projects/{id}/delivery",
            get(projects::delivery).post(projects::deliver),
        )
        .route(
            "/api/projects/{id}/messages",
            get(messages::list).post(messages::send),
        )
        .route(
            "/api/projects/{id}/tasks",
            get(tasks::list).post(tasks::dispatch),
        )
        .route("/api/projects/{id}/tasks/{task_id}", patch(tasks::reorder))
        .route("/api/runs/{id}", get(runs::get))
        .route("/api/approvals", get(approvals::pending))
        .route(
            "/api/approvals/{id}",
            get(approvals::get).post(approvals::decide),
        )
        .route("/api/projects/{id}/spec-versions", get(specs::list))
        .route("/api/spec-versions/{id}/approve", post(specs::approve))
        .route("/api/spec-versions/{id}/validation", get(specs::validate))
        .route(
            "/api/spec-versions/{id}/artifacts/{*path}",
            get(specs::artifact),
        )
        .route("/api/roles", get(roles))
        .route("/api/agent-auth/start", post(agent_auth::start))
        .route("/api/agent-auth/status", get(agent_auth::status_all))
        .route("/api/agent-auth/disconnect", post(agent_auth::disconnect))
        .route(
            "/api/settings/routing",
            get(settings::get_routing).put(settings::put_routing),
        )
        .route("/api/agent-auth/{id}", get(agent_auth::status))
        .route("/api/agent-auth/{id}/code", post(agent_auth::submit_code))
        .route("/api/github/repos", get(github::repos))
        .route("/api/events", get(events::stream))
        .route(
            "/api/projects/{id}/preview",
            get(preview::status)
                .post(preview::ensure)
                .delete(preview::stop),
        )
        .route("/api/projects/{id}/preview/", any(preview::proxy_root))
        .route("/api/projects/{id}/preview/{*path}", any(preview::proxy))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

    Router::new()
        .route("/api/health", get(health))
        .merge(protected)
        // The web UI: embedded assets with SPA fallback (see assets.rs).
        .fallback(crate::assets::static_or_spa)
        .with_state(state)
}

async fn health(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.store.health().await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "database": "ok"})),
        ),
        Err(error) => {
            tracing::error!(error = %error, "health database probe failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "unavailable",
                    "database": "unavailable"
                })),
            )
        }
    }
}

/// The fixed team plus each role's skill path (spec §5.3 `/api/roles`).
async fn roles(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Vec<dto::RoleDto>>, crate::error::ApiError> {
    let roles = state.store.list_roles().await?;
    Ok(Json(roles.iter().map(dto::RoleDto::from).collect()))
}
