//! Authenticated trusted comparison metadata and binary artifacts.

use super::dto::VisualComparisonDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
use latoile_core::ids::{RunId, VisualComparisonId};
use latoile_core::ports::{RunStore, VisualComparisonRenderer, VisualComparisonStore};

pub async fn list_for_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<VisualComparisonDto>>, ApiError> {
    let id = RunId::new(id).map_err(|error| ApiError::bad_request(error.to_string()))?;
    if RunStore::get(&state.store, &id).await?.is_none() {
        return Err(ApiError::not_found("run"));
    }
    let rows = VisualComparisonStore::list_for_run(&state.store, &id).await?;
    Ok(Json(rows.iter().map(VisualComparisonDto::from).collect()))
}

pub async fn render_png(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    artifact(state, id, Artifact::Render).await
}

pub async fn heatmap_png(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    artifact(state, id, Artifact::Heatmap).await
}

enum Artifact {
    Render,
    Heatmap,
}

async fn artifact(
    state: AppState,
    id: String,
    artifact: Artifact,
) -> Result<impl IntoResponse, ApiError> {
    let id =
        VisualComparisonId::new(id).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let comparison = VisualComparisonStore::get(&state.store, &id)
        .await?
        .ok_or(ApiError::not_found("visual comparison"))?;
    if !comparison.status.has_trusted_evidence() {
        return Err(ApiError::not_found("visual comparison artifact"));
    }
    let (bytes, digest) = match artifact {
        Artifact::Render => (
            state.baselines.read_render_png(&comparison).await?,
            comparison.render_png_digest.as_deref(),
        ),
        Artifact::Heatmap => (
            state.baselines.read_heatmap_png(&comparison).await?,
            comparison.heatmap_png_digest.as_deref(),
        ),
    };
    let digest = digest.ok_or(ApiError::not_found("visual comparison artifact"))?;
    let etag = HeaderValue::from_str(&format!("\"sha256:{digest}\""))
        .map_err(|_| ApiError::bad_request("invalid visual evidence digest"))?;
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=31536000, immutable"),
            ),
            (header::ETAG, etag),
        ],
        bytes,
    ))
}
