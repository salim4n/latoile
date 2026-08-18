//! Owner-visible architecture discovery state. Conversation writes still go
//! through `/messages`; this route exposes status/history and cancellation so
//! the chat never hides a lost or blocked Architect session.

use super::dto::ArchitectureSessionDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use latoile_app::use_cases::CancelArchitecture;
use latoile_core::ids::ProjectId;
use latoile_core::ports::ArchitectureSessionStore;

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<ArchitectureSessionDto>>, ApiError> {
    let project = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let Some(session) = state.store.latest_for_project(&project).await? else {
        return Ok(Json(None));
    };
    let questions = state.store.questions_for_session(&session.id).await?;
    Ok(Json(Some(ArchitectureSessionDto::new(
        &session, &questions,
    ))))
}

pub async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ArchitectureSessionDto>, ApiError> {
    let project = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let session = CancelArchitecture::new(state.store.clone(), state.agents.clone())
        .execute(&project)
        .await?;
    let questions = state.store.questions_for_session(&session.id).await?;
    Ok(Json(ArchitectureSessionDto::new(&session, &questions)))
}
