//! `/api/approvals` — the inbox. `GET` lists what waits on the owner;
//! `POST /:id` decides. A granted review approval may close its task — that
//! orchestration is the use cases', not this file's.

use super::dto::ApprovalDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use latoile_app::use_cases::{DecidePermission, GrantApproval, RejectApproval};
use latoile_core::ids::ApprovalId;
use latoile_core::ports::ApprovalStore;
use latoile_core::ApprovalKind;
use serde::Deserialize;

pub async fn pending(State(state): State<AppState>) -> Result<Json<Vec<ApprovalDto>>, ApiError> {
    let approvals = state.store.list_pending_for_inbox().await?;
    Ok(Json(approvals.iter().map(ApprovalDto::from).collect()))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApprovalDto>, ApiError> {
    let id = ApprovalId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let approval = state
        .store
        .approval_detail(&id)
        .await?
        .ok_or(ApiError::not_found("approval"))?;
    Ok(Json(ApprovalDto::from(&approval)))
}

#[derive(Deserialize)]
pub struct DecideBody {
    granted: bool,
    comment: Option<String>,
}

pub async fn decide(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<DecideBody>,
) -> Result<Json<ApprovalDto>, ApiError> {
    let id = ApprovalId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let _decision_guard = state.decision_lock.lock().await;
    let current = ApprovalStore::get(&state.store, &id)
        .await?
        .ok_or(ApiError::not_found("approval"))?;
    if current.kind == ApprovalKind::Permission {
        let approval = DecidePermission::new(
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
            state.agents.clone(),
        )
        .execute(&id, body.granted, body.comment)
        .await?;
        return Ok(Json(ApprovalDto::from(&approval)));
    }
    let approval = if body.granted {
        GrantApproval::new(
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
        )
        .execute_with_comment(&id, body.comment)
        .await?
        .approval
    } else {
        RejectApproval::new(
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
            state.store.clone(),
            state.agents.clone(),
        )
        .execute_with_comment(&id, body.comment)
        .await?
    };
    Ok(Json(ApprovalDto::from(&approval)))
}
