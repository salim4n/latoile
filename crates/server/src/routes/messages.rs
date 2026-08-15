//! `/api/projects/:id/messages` — the Manager thread. POST persists the
//! owner's message (SendMessage), then runs the manager turn inline and
//! persists the reply as a Manager message. This is the smallest honest
//! wiring: the manager's structured `actions` are stored but NOT executed —
//! turning them into tasks/runs is the orchestrator pass.

use super::dto::MessageDto;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use latoile_app::use_cases::{SendMessage, SendMessageInput};
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{MessageId, ProjectId};
use latoile_core::ports::{AgentChannel, ConversationStore, EventLog};
use latoile_core::{Author, Message};
use serde::{Deserialize, Serialize};

pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<MessageDto>>, ApiError> {
    let id = ProjectId::new(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let messages = state.store.recent(&id, params.limit.unwrap_or(50)).await?;
    Ok(Json(messages.iter().map(MessageDto::from).collect()))
}

#[derive(Deserialize)]
pub struct ListParams {
    limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct SendBody {
    content: String,
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

    let reply = match state.agents.tell_manager(&project_id, &body.content).await {
        Ok(reply) if !reply.content.trim().is_empty() => {
            Some(persist_reply(&state, &project_id, reply).await?)
        }
        Ok(_) => None,
        Err(e) => {
            // Surfaced in the log, not in the response: the owner's message
            // succeeded, and that is what the response describes.
            tracing::warn!(error = %e, "the manager did not answer");
            None
        }
    };

    Ok(Json(SendResponse {
        message: MessageDto::from(&posted.message),
        reply: reply.map(|m| MessageDto::from(&m)),
    }))
}

/// Persist the Manager's answer as a thread message, with its structured
/// actions attached, and journal it.
async fn persist_reply(
    state: &AppState,
    project_id: &ProjectId,
    reply: latoile_core::ports::ManagerReply,
) -> Result<Message, ApiError> {
    let conversation = state
        .store
        .for_project(project_id)
        .await?
        .ok_or(ApiError::not_found("conversation"))?;
    let message = Message::new(
        MessageId::new(ulid::Ulid::new().to_string())
            .map_err(|e| ApiError::bad_request(e.to_string()))?,
        conversation.id,
        Author::Manager,
        reply.content,
        reply.actions,
    )
    .map_err(ApiError::domain)?;
    ConversationStore::append(&state.store, &message).await?;
    EventLog::append(
        &state.store,
        &NewEvent {
            project_id: project_id.clone(),
            kind: EventKind::MessagePosted,
            payload: format!("{{\"message_id\":\"{}\"}}", message.id),
        },
    )
    .await?;
    Ok(message)
}
