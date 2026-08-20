//! `/api/github/repos` — the repo picker. The token comes from the vault via
//! the GitHub adapter; a missing token surfaces as the adapter's auth error.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use latoile_core::ports::GitHubClient;
use serde::Serialize;

#[derive(Serialize)]
pub struct RepoDto {
    full_name: String,
    description: Option<String>,
    private: bool,
}

pub async fn repos(State(state): State<AppState>) -> Result<Json<Vec<RepoDto>>, ApiError> {
    let repos = state.github.list_repos().await?;
    Ok(Json(
        repos
            .into_iter()
            .map(|r| RepoDto {
                full_name: r.full_name,
                description: r.description,
                private: r.private,
            })
            .collect(),
    ))
}
