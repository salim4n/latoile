//! `/api/projects/:id/tasks` — the board. `POST` dispatches a task through
//! the DispatchTask use case (spec-before-code is enforced there). `PATCH`
//! is reordering only: position is plain data, not a state transition, so it
//! is a direct store write — the one documented handler-side mutation.

use super::dto::TaskDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use latoile_app::use_cases::{DispatchTask, DispatchTaskInput};
use latoile_core::TriggeredBy;
use latoile_core::ids::{ProjectId, RoleId, TaskId};
use latoile_core::ports::TaskStore;
use serde::Deserialize;

pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TaskDto>>, ApiError> {
    let id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let tasks = state.store.list_project_task_rows(&id).await?;
    Ok(Json(tasks.iter().map(TaskDto::from).collect()))
}

#[derive(Deserialize)]
pub struct DispatchBody {
    role_id: String,
    title: String,
    #[serde(default)]
    description: String,
    prompt: Option<String>,
}

pub async fn dispatch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<DispatchBody>,
) -> Result<Json<TaskDto>, ApiError> {
    let project_id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let position = state.store.list_for_project(&project_id).await?.len() as u32;
    let prompt = body.prompt.unwrap_or_else(|| body.description.clone());

    let dispatched = DispatchTask::new(
        state.store.clone(),
        state.store.clone(),
        state.store.clone(),
        state.store.clone(),
        state.baselines.clone(),
        state.agents.clone(),
        state.store.clone(),
    )
    .execute(DispatchTaskInput {
        project_id,
        role_id: RoleId::new(body.role_id).map_err(|e| ApiError::bad_request(e.to_string()))?,
        title: body.title,
        description: body.description,
        position,
        triggered_by: TriggeredBy::User,
        prompt,
    })
    .await?;
    Ok(Json(TaskDto::from(&dispatched.task)))
}

#[derive(Deserialize)]
pub struct ReorderBody {
    position: u32,
}

pub async fn reorder(
    State(state): State<AppState>,
    Path((id, task_id)): Path<(String, String)>,
    Json(body): Json<ReorderBody>,
) -> Result<Json<TaskDto>, ApiError> {
    let _ = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let task_id = TaskId::new(task_id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let mut task = TaskStore::get(&state.store, &task_id)
        .await?
        .ok_or(ApiError::not_found("task"))?;
    task.position = body.position;
    TaskStore::save(&state.store, &task).await?;
    Ok(Json(TaskDto::from(&task)))
}
