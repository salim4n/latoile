//! `/api/runs/:id` — run detail with sanitized summary, Git SHAs and bounded
//! artifact metadata. Raw diffs stay in Git; the Reviewer supplies the
//! owner-facing excerpt in its approval payload.

use super::dto::RunDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use latoile_core::ids::RunId;
use latoile_core::ports::RunStore;

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunDto>, ApiError> {
    let id = RunId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let run = RunStore::get(&state.store, &id)
        .await?
        .ok_or(ApiError::not_found("run"))?;
    Ok(Json(RunDto::from(&run)))
}
