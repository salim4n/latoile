//! Start one persistent Socratic Architect discovery session from the
//! project's initial brief. The user message is already durable; this use
//! case owns the session, first validated turn and owner-visible reply.

use super::UseCaseError;
use crate::architecture_turn::{parse_architecture_turn, ArchitectureTurn, ArchitectureTurnKind};
use crate::store::Store;
use latoile_core::ids::{ArchitectureQuestionId, ArchitectureSessionId, MessageId, ProjectId};
use latoile_core::ports::{
    AgentChannel, ArchitectureSessionStore, ConversationStore, EventLog, ProjectStore,
};
use latoile_core::{
    ArchitectureQuestion, ArchitectureSession, Author, EventKind, Message, NewEvent,
};

pub struct ArchitectureOutcome {
    pub session: ArchitectureSession,
    pub question: Option<ArchitectureQuestion>,
    pub message: Message,
}

pub struct StartArchitecture<A> {
    store: Store,
    agents: A,
}

impl<A: AgentChannel> StartArchitecture<A> {
    pub fn new(store: Store, agents: A) -> Self {
        Self { store, agents }
    }

    pub async fn execute(
        &self,
        project: &ProjectId,
        brief: &str,
    ) -> Result<ArchitectureOutcome, UseCaseError> {
        if brief.trim().is_empty() {
            return Err(latoile_core::DomainError::Invariant(
                "architecture discovery needs the owner's initial brief",
            )
            .into());
        }
        ProjectStore::get(&self.store, project)
            .await?
            .ok_or(UseCaseError::NotFound("project"))?;
        if self.store.active_for_project(project).await?.is_some() {
            return Err(latoile_core::DomainError::Invariant(
                "a project already has an active architecture session",
            )
            .into());
        }

        let mut session = ArchitectureSession::new(
            ArchitectureSessionId::new(ulid::Ulid::new().to_string())?,
            project.clone(),
        );
        ArchitectureSessionStore::save(&self.store, &session).await?;

        let mut reply = match self
            .agents
            .start_architecture(project, &session.id, brief)
            .await
        {
            Ok(reply) => reply,
            Err(error) => {
                session.fail("Architect provider session could not start; retry discovery")?;
                ArchitectureSessionStore::save(&self.store, &session).await?;
                return Err(error.into());
            }
        };
        if let Err(error) = session
            .attach_agent(reply.acp_session_id.clone())
            .and_then(|_| {
                session.record_skill(
                    reply.skill_name.clone(),
                    reply.skill_digest.clone(),
                    reply.operating_mode,
                )
            })
        {
            session.fail("Architect provider returned invalid session or skill provenance")?;
            ArchitectureSessionStore::save(&self.store, &session).await?;
            return Err(error.into());
        }
        let mut turn = match parse_architecture_turn(&reply.content) {
            Ok(turn) => turn,
            Err(reason) => {
                session.fail(reason)?;
                ArchitectureSessionStore::save(&self.store, &session).await?;
                return Err(latoile_core::ports::PortError(reason.into()).into());
            }
        };
        if turn.kind == ArchitectureTurnKind::ReadyToDraft {
            reply = match self
                .agents
                .retry_architecture_question(project, &session.id)
                .await
            {
                Ok(reply) => reply,
                Err(error) => {
                    session.fail(
                        "Architect skipped the mandatory first challenge and could not be recentered",
                    )?;
                    ArchitectureSessionStore::save(&self.store, &session).await?;
                    return Err(error.into());
                }
            };
            if session.acp_session_id.as_deref() != Some(reply.acp_session_id.as_str())
                || session.skill_name.as_deref() != Some(reply.skill_name.as_str())
                || session.skill_digest.as_deref() != Some(reply.skill_digest.as_str())
                || session.operating_mode != Some(reply.operating_mode)
            {
                let reason =
                    "Architect context or pinned skill provenance changed during discovery guard";
                session.fail(reason)?;
                ArchitectureSessionStore::save(&self.store, &session).await?;
                return Err(latoile_core::ports::PortError(reason.into()).into());
            }
            turn = match parse_architecture_turn(&reply.content) {
                Ok(turn) => turn,
                Err(reason) => {
                    session.fail(reason)?;
                    ArchitectureSessionStore::save(&self.store, &session).await?;
                    return Err(latoile_core::ports::PortError(reason.into()).into());
                }
            };
            if turn.kind == ArchitectureTurnKind::ReadyToDraft {
                let reason = "the Architect ignored the mandatory first owner challenge twice";
                session.fail(reason)?;
                ArchitectureSessionStore::save(&self.store, &session).await?;
                return Err(latoile_core::DomainError::Invariant(reason).into());
            }
        }
        if let Err(error) = apply_turn(&mut session, &turn) {
            session.fail(format!("invalid Architect discovery transition: {error}"))?;
            ArchitectureSessionStore::save(&self.store, &session).await?;
            return Err(error);
        }
        let question = question_for(&session, &turn, 1)?;
        self.store
            .save_turn(&session, None, question.as_ref())
            .await?;
        let message = persist_architect_message(&self.store, project, &session, &turn).await?;
        Ok(ArchitectureOutcome {
            session,
            question,
            message,
        })
    }
}

pub(crate) fn apply_turn(
    session: &mut ArchitectureSession,
    turn: &ArchitectureTurn,
) -> Result<(), UseCaseError> {
    match turn.kind {
        ArchitectureTurnKind::Question => session.ask(turn.phase)?,
        ArchitectureTurnKind::ReadyToDraft => session.ready_to_draft()?,
    }
    Ok(())
}

pub(crate) fn question_for(
    session: &ArchitectureSession,
    turn: &ArchitectureTurn,
    sequence: u32,
) -> Result<Option<ArchitectureQuestion>, UseCaseError> {
    match turn.kind {
        ArchitectureTurnKind::Question => Ok(Some(ArchitectureQuestion::new(
            ArchitectureQuestionId::new(ulid::Ulid::new().to_string())?,
            session.id.clone(),
            sequence,
            turn.message.clone(),
        )?)),
        ArchitectureTurnKind::ReadyToDraft => Ok(None),
    }
}

pub(crate) async fn persist_architect_message(
    store: &Store,
    project: &ProjectId,
    session: &ArchitectureSession,
    turn: &ArchitectureTurn,
) -> Result<Message, UseCaseError> {
    let conversation = store
        .for_project(project)
        .await?
        .ok_or(UseCaseError::NotFound("conversation"))?;
    let kind = match turn.kind {
        ArchitectureTurnKind::Question => "question",
        ArchitectureTurnKind::ReadyToDraft => "ready_to_draft",
    };
    let actions = serde_json::json!([{
        "type": "architecture",
        "kind": kind,
        "sub": turn.message,
        "session_id": session.id.as_str(),
        "phase": session.phase.as_str(),
        "status": session.status.as_str(),
    }]);
    let message = Message::new(
        MessageId::new(ulid::Ulid::new().to_string())?,
        conversation.id,
        Author::Manager,
        turn.message.clone(),
        Some(actions.to_string()),
    )?;
    ConversationStore::append(store, &message).await?;
    EventLog::append(
        store,
        &NewEvent {
            project_id: project.clone(),
            kind: EventKind::MessagePosted,
            payload: format!("{{\"message_id\":\"{}\"}}", message.id),
        },
    )
    .await?;
    Ok(message)
}
