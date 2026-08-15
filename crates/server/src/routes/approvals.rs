//! `/api/approvals` — the inbox. `GET` lists what waits on the owner;
//! `POST /:id` decides. A granted review approval may close its task — that
//! orchestration is the use cases', not this file's.

use super::dto::ApprovalDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use latoile_app::use_cases::{GrantApproval, RejectApproval};
use latoile_core::ids::ApprovalId;
use latoile_core::ports::ApprovalStore;
use serde::Deserialize;

pub async fn pending(State(state): State<AppState>) -> Result<Json<Vec<ApprovalDto>>, ApiError> {
    let approvals = state.store.list_pending().await?;
    Ok(Json(approvals.iter().map(ApprovalDto::from).collect()))
}

#[derive(Deserialize)]
pub struct DecideBody {
    granted: bool,
    /// Accepted for forward-compatibility; the domain has nowhere to keep
    /// it yet (Approval carries only the request payload).
    #[allow(dead_code)]
    comment: Option<String>,
}

pub async fn decide(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<DecideBody>,
) -> Result<Json<ApprovalDto>, ApiError> {
    let id = ApprovalId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let approval = if body.granted {
        GrantApproval::new(
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
        )
        .execute(&id)
        .await?
        .approval
    } else {
        RejectApproval::new(
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
        )
        .execute(&id)
        .await?
    };
    Ok(Json(ApprovalDto::from(&approval)))
}
