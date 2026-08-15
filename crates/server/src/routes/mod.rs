//! The router. Every handler extracts, validates, delegates — no decisions
//! live here (contract §2, guardian #5). `/api/health` is the only open
//! route; everything else sits behind the bearer token (D9).

pub mod dto;
mod approvals;
mod events;
mod github;
mod messages;
mod preview;
mod projects;
mod runs;
mod specs;
mod tasks;

use crate::auth;
use crate::state::AppState;
use axum::routing::{any, get, patch, post};
use axum::{middleware, Json, Router};

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/projects", get(projects::list).post(projects::create))
        .route("/api/projects/{id}", get(projects::get))
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
        .route("/api/approvals/{id}", post(approvals::decide))
        .route("/api/projects/{id}/spec-versions", get(specs::list))
        .route("/api/spec-versions/{id}/approve", post(specs::approve))
        .route("/api/roles", get(roles))
        .route("/api/github/repos", get(github::repos))
        .route("/api/events", get(events::stream))
        .route(
            "/api/projects/{id}/preview",
            get(preview::status).post(preview::ensure).delete(preview::stop),
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
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

/// The fixed team plus each role's skill path (spec §5.3 `/api/roles`).
async fn roles(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Vec<dto::RoleDto>>, crate::error::ApiError> {
    let roles = state.store.list_roles().await?;
    Ok(Json(roles.iter().map(dto::RoleDto::from).collect()))
}
