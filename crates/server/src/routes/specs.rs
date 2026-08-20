//! Spec versions: the project's list, and approval. The list route is an
//! addition to the contract — §5.3 defines approve-by-id but no way to learn
//! the id; the UI needs the list to render the approve button.

use super::dto::{ArchitecturePackageValidationDto, SpecDto, VisualBaselineDto};
use crate::error::ApiError;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
use latoile_app::use_cases::{ApproveSpec, CaptureBaselines};
use latoile_core::ids::{ProjectId, SpecVersionId};
use latoile_core::ports::{AgentChannel, VisualBaselineRenderer, VisualBaselineStore};
use latoile_core::{DomainError, VisualBaselineStatus};

pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SpecDto>>, ApiError> {
    let id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let specs = state.store.specs_for_project(&id).await?;
    Ok(Json(specs.iter().map(SpecDto::from).collect()))
}

pub async fn approve(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SpecDto>, ApiError> {
    let id = SpecVersionId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let _decision_guard = state.decision_lock.lock().await;
    let baselines = CaptureBaselines::new(
        state.store.clone(),
        state.agents.clone(),
        state.baselines.clone(),
    )
    .execute(&id)
    .await?;
    if baselines
        .iter()
        .any(|baseline| baseline.status != VisualBaselineStatus::Ready)
    {
        return Err(
            latoile_app::use_cases::UseCaseError::from(DomainError::Invariant(
                "every required visual baseline must be ready before approval",
            ))
            .into(),
        );
    }
    let spec = ApproveSpec::new(state.store.clone(), state.agents.clone())
        .execute(&id)
        .await?;
    Ok(Json(SpecDto::from(&spec)))
}

pub async fn baselines(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<VisualBaselineDto>>, ApiError> {
    let id = SpecVersionId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let rows = VisualBaselineStore::list_for_spec(&state.store, &id).await?;
    Ok(Json(rows.iter().map(VisualBaselineDto::from).collect()))
}

pub async fn capture_baselines(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<VisualBaselineDto>>, ApiError> {
    let id = SpecVersionId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let _decision_guard = state.decision_lock.lock().await;
    let rows = CaptureBaselines::new(
        state.store.clone(),
        state.agents.clone(),
        state.baselines.clone(),
    )
    .execute(&id)
    .await?;
    Ok(Json(rows.iter().map(VisualBaselineDto::from).collect()))
}

pub async fn baseline_png(
    State(state): State<AppState>,
    Path((id, comparison_id)): Path<(String, String)>,
) -> Result<axum::response::Response, ApiError> {
    let id = SpecVersionId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let baseline = VisualBaselineStore::get(&state.store, &id, &comparison_id)
        .await?
        .ok_or(ApiError::not_found("visual baseline"))?;
    if baseline.status != VisualBaselineStatus::Ready {
        return Err(ApiError::not_found("visual baseline PNG"));
    }
    let png = state.baselines.read_png(&baseline).await?;
    let etag = HeaderValue::from_str(&format!(
        "\"{}\"",
        baseline.png_digest.as_deref().unwrap_or_default()
    ))
    .map_err(|_| ApiError::bad_request("invalid baseline digest"))?;
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=31536000, immutable"),
            ),
            (header::ETAG, etag),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        ],
        png,
    )
        .into_response())
}

pub async fn validate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ArchitecturePackageValidationDto>, ApiError> {
    let id = SpecVersionId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let spec = state
        .store
        .spec_by_id(&id)
        .await?
        .ok_or(ApiError::not_found("spec version"))?;
    let validation = state
        .agents
        .verify_architecture_package(&spec.project_id, &spec)
        .await?;
    Ok(Json(ArchitecturePackageValidationDto::from(&validation)))
}

pub async fn artifact(
    State(state): State<AppState>,
    Path((id, path)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SpecVersionId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    if std::path::Path::new(&path)
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("html")
    {
        return Err(ApiError::bad_request(
            "only static HTML architecture artifacts can be rendered",
        ));
    }
    let spec = state
        .store
        .spec_by_id(&id)
        .await?
        .ok_or(ApiError::not_found("spec version"))?;
    let html = state
        .agents
        .read_architecture_artifact(&spec.project_id, &spec, &path)
        .await?;
    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        ),
        (
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'self'",
            ),
        ),
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
    ];
    Ok((headers, html))
}
