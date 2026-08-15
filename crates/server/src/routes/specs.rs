//! Spec versions: the project's list, and approval. The list route is an
//! addition to the contract — §5.3 defines approve-by-id but no way to learn
//! the id; the UI needs the list to render the approve button.

use super::dto::SpecDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use latoile_app::use_cases::ApproveSpec;
use latoile_core::ids::{ProjectId, SpecVersionId};

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
    let spec = ApproveSpec::new(state.store.clone()).execute(&id).await?;
    Ok(Json(SpecDto::from(&spec)))
}
