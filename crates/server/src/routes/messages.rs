//! `/api/projects/:id/messages` — the Manager thread. POST persists the
//! owner's message (SendMessage), then runs the manager turn inline:
//! the reply's actions block executes (ManagerTurn — tasks, runs, specs)
//! and the reply is persisted with its display cards.

use super::dto::MessageDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use latoile_app::use_cases::{
    AnswerArchitecture, ManagerTurn, SendMessage, SendMessageInput, StartArchitecture,
};
use latoile_core::ids::ProjectId;
use latoile_core::ports::{AgentChannel, ArchitectureSessionStore};
use latoile_core::ArchitectureStatus;
use serde::{Deserialize, Serialize};

pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<MessageDto>>, ApiError> {
    let id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let messages = state
        .store
        .recent_message_rows(&id, params.limit.unwrap_or(50))
        .await?;
    Ok(Json(messages.iter().map(MessageDto::from).collect()))
}

#[derive(Deserialize)]
pub struct ListParams {
    limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct SendBody {
    content: String,
    #[serde(default)]
    intent: Option<String>,
}

#[derive(Serialize)]
pub struct SendResponse {
    pub message: MessageDto,
    /// The Manager's reply. `null` when the Manager is unreachable — the
    /// owner's message is already persisted either way; a dead agent must
    /// not eat it.
    pub reply: Option<MessageDto>,
}

pub async fn send(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SendBody>,
) -> Result<Json<SendResponse>, ApiError> {
    let project_id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;

    // The owner's message first: it is durable before any agent is woken.
    let posted = SendMessage::new(state.store.clone(), state.store.clone())
        .execute(SendMessageInput {
            project_id: project_id.clone(),
            content: body.content.clone(),
        })
        .await?;

    let active_architecture = state.store.active_for_project(&project_id).await?;
    let architecture_reply = if body.intent.as_deref() == Some("architecture_brief") {
        Some(
            StartArchitecture::new(state.store.clone(), state.agents.clone())
                .execute(&project_id, &body.content)
                .await?
                .message,
        )
    } else if active_architecture
        .as_ref()
        .is_some_and(|session| session.status == ArchitectureStatus::AwaitingAnswer)
    {
        Some(
            AnswerArchitecture::new(state.store.clone(), state.agents.clone())
                .execute(&project_id, &body.content)
                .await?
                .message,
        )
    } else {
        None
    };

    let reply = if let Some(reply) = architecture_reply {
        Some(MessageDto::from(&reply))
    } else {
        match state.agents.tell_manager(&project_id, &body.content).await {
            Ok(reply) if !reply.content.trim().is_empty() => {
                // The Manager's actions execute here — tasks appear on the
                // board, runs start, specs draft — before the reply renders.
                let outcome = ManagerTurn::new(state.store.clone(), state.agents.clone())
                    .record_reply(&project_id, reply)
                    .await?;
                Some(MessageDto::from(&outcome.message))
            }
            Ok(_) => None,
            Err(e) => {
                // Surfaced in the log, not in the response: the owner's message
                // succeeded, and that is what the response describes.
                tracing::warn!(error = %e, "the manager did not answer");
                None
            }
        }
    };

    Ok(Json(SendResponse {
        message: MessageDto::from(&posted.message),
        reply,
    }))
}
