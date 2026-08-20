//! `/api/projects/:id/preview` — status, ensure, stop — and the reverse
//! proxy under `/preview/*` (spec §5.1: the proxy lives here, not in the
//! preview crate). The UI iframe hits one origin; LaToile forwards to the
//! dev server's loopback port. Token-gated like everything else (D9).
//!
//! Bodies stream in both directions — nothing is buffered, so HMR payloads
//! and large bundles pass through untouched. WebSocket upgrade (vite's HMR
//! channel) is the known V1 gap; D10's reload is SSE-driven, which is plain
//! HTTP and traverses fine.

use super::dto::PreviewDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use latoile_app::use_cases::{EnsurePreview, StopPreview};
use latoile_core::ids::ProjectId;
use latoile_core::ports::{PreviewStore, ProjectStore};

/// No preview running is not an error state for the UI — it is a 404 with
/// the contract's shape.
fn no_preview() -> ApiError {
    ApiError::not_found("preview")
}

pub async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<PreviewDto>>, ApiError> {
    let id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    if state.store.get(&id).await?.is_none() {
        return Err(ApiError::not_found("project"));
    }
    let preview = state.store.active_for_project(&id).await?;
    let dto = match preview {
        Some(preview) => {
            let alive = state.previews.is_alive(&preview.id).await;
            let logs = state.previews.logs(&preview.id).await;
            Some(PreviewDto::of(&preview, alive, logs))
        }
        None => None,
    };
    Ok(Json(dto))
}

pub async fn ensure(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PreviewDto>, ApiError> {
    let id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let ensured = EnsurePreview::new(
        state.store.clone(),
        state.store.clone(),
        state.previews.clone(),
        state.store.clone(),
    )
    .execute(&id)
    .await?;
    let logs = state.previews.logs(&ensured.preview.id).await;
    Ok(Json(PreviewDto::of(&ensured.preview, true, logs)))
}

pub async fn stop(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    StopPreview::new(state.store.clone(), state.previews.clone())
        .execute(&id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Hop-by-hop headers never cross a proxy in either direction.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
];

/// `/preview/` with nothing after it — the iframe's root hit. The wildcard
/// route below does not match an empty path, so this one exists.
pub async fn proxy_root(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    proxy_to(&state, &id, "", request).await
}

pub async fn proxy(
    State(state): State<AppState>,
    Path((id, path)): Path<(String, String)>,
    request: Request,
) -> Result<Response, ApiError> {
    proxy_to(&state, &id, &path, request).await
}

async fn proxy_to(
    state: &AppState,
    id: &str,
    path: &str,
    request: Request,
) -> Result<Response, ApiError> {
    let id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let preview = state
        .store
        .active_for_project(&id)
        .await?
        .ok_or_else(no_preview)?;

    let query = request
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let url = format!("http://127.0.0.1:{}/{path}{query}", preview.port);

    let method = request.method().clone();
    let headers = request.headers().clone();
    let body = request.into_body();
    let mut outbound = state
        .proxy_http
        .request(method, &url)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()));
    for (name, value) in &headers {
        if !HOP_BY_HOP.contains(&name.as_str()) && name != "content-length" && name != "host" {
            outbound = outbound.header(name, value);
        }
    }

    let upstream = outbound.send().await.map_err(|e| {
        ApiError::internal(latoile_core::ports::PortError(format!(
            "preview upstream: {e}"
        )))
    })?;

    let mut response = Response::builder().status(upstream.status());
    if let Some(map) = response.headers_mut() {
        copy_headers(upstream.headers(), map);
    }
    let body = Body::from_stream(upstream.bytes_stream());
    response
        .body(body)
        .map_err(|e| ApiError::bad_request(e.to_string()))
}

fn copy_headers(from: &HeaderMap, to: &mut HeaderMap) {
    for (name, value) in from {
        if !HOP_BY_HOP.contains(&name.as_str()) && name != "content-length" {
            to.append(name, value.clone());
        }
    }
}
