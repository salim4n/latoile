//! `/api/projects` — list, create, detail.

use super::dto::ProjectDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use latoile_app::use_cases::{CreateProject, CreateProjectInput};
use latoile_core::ids::ProjectId;
use latoile_core::ports::ProjectStore;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CreateProjectBody {
    name: String,
    slug: String,
    github_repo: String,
    #[serde(default = "default_work_branch")]
    work_branch: String,
    #[serde(default)]
    dev_command: Option<String>,
}

fn default_work_branch() -> String {
    "work".into()
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<ProjectDto>>, ApiError> {
    let projects = state.store.list_project_rows().await?;
    Ok(Json(projects.iter().map(ProjectDto::from).collect()))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateProjectBody>,
) -> Result<Json<ProjectDto>, ApiError> {
    let project = CreateProject::new(state.store.clone(), state.github.clone())
        .execute(CreateProjectInput {
            name: body.name,
            slug: body.slug,
            github_repo: body.github_repo,
            work_branch: body.work_branch,
            dev_command: body.dev_command,
        })
        .await?;
    Ok(Json(ProjectDto::from(&project)))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectDto>, ApiError> {
    let id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let project = ProjectStore::get(&state.store, &id)
        .await?
        .filter(|p| !p.deleted)
        .ok_or(ApiError::not_found("project"))?;
    Ok(Json(ProjectDto::from(&project)))
}
