//! Route one owner answer from the Manager surface back into the live
//! Architect session. The answer is committed before the provider is called;
//! a failed process never eats the owner's decision.

use super::produce_architecture_package::produce_architecture_package;
use super::start_architecture::{
    apply_turn, persist_architect_message, question_for, ArchitectureOutcome,
};
use super::UseCaseError;
use crate::architecture_turn::{parse_architecture_turn, ArchitectureTurnKind};
use crate::store::Store;
use latoile_core::ids::ProjectId;
use latoile_core::ports::{AgentChannel, ArchitectureSessionStore};
use latoile_core::ArchitectureStatus;

pub struct AnswerArchitecture<A> {
    store: Store,
    agents: A,
}

impl<A: AgentChannel> AnswerArchitecture<A> {
    pub fn new(store: Store, agents: A) -> Self {
        Self { store, agents }
    }

    pub async fn execute(
        &self,
        project: &ProjectId,
        answer: &str,
    ) -> Result<ArchitectureOutcome, UseCaseError> {
        let mut session = self
            .store
            .active_for_project(project)
            .await?
            .ok_or(UseCaseError::NotFound("active architecture session"))?;
        if session.status != ArchitectureStatus::AwaitingAnswer {
            return Err(latoile_core::DomainError::Invariant(
                "the Architect is not waiting for an owner answer",
            )
            .into());
        }
        let mut answered = self
            .store
            .open_question(&session.id)
            .await?
            .ok_or(UseCaseError::NotFound("open architecture question"))?;
        answered.answer(answer)?;
        session.receive_answer()?;
        self.store
            .save_turn(&session, Some(&answered), None)
            .await?;

        let reply = match self
            .agents
            .continue_architecture(project, &session.id, answer)
            .await
        {
            Ok(reply) => reply,
            Err(error) => {
                session.fail("live Architect session was lost; restart discovery")?;
                ArchitectureSessionStore::save(&self.store, &session).await?;
                return Err(error.into());
            }
        };
        if session.acp_session_id.as_deref() != Some(reply.acp_session_id.as_str())
            || session.skill_name.as_deref() != Some(reply.skill_name.as_str())
            || session.skill_digest.as_deref() != Some(reply.skill_digest.as_str())
            || session.operating_mode != Some(reply.operating_mode)
        {
            session
                .fail("live Architect context or pinned skill provenance changed mid-discovery")?;
            ArchitectureSessionStore::save(&self.store, &session).await?;
            return Err(latoile_core::ports::PortError(
                "live Architect context changed mid-discovery".into(),
            )
            .into());
        }
        let turn = match parse_architecture_turn(&reply.content) {
            Ok(turn) => turn,
            Err(reason) => {
                session.fail(reason)?;
                ArchitectureSessionStore::save(&self.store, &session).await?;
                return Err(latoile_core::ports::PortError(reason.into()).into());
            }
        };
        if let Err(error) = apply_turn(&mut session, &turn) {
            session.fail(format!("invalid Architect discovery transition: {error}"))?;
            ArchitectureSessionStore::save(&self.store, &session).await?;
            return Err(error);
        }
        let sequence = self.store.questions_for_session(&session.id).await?.len() as u32 + 1;
        let question = question_for(&session, &turn, sequence)?;
        self.store
            .save_turn(&session, None, question.as_ref())
            .await?;
        let mut message = persist_architect_message(&self.store, project, &session, &turn).await?;
        if turn.kind == ArchitectureTurnKind::ReadyToDraft {
            message =
                produce_architecture_package(&self.store, &self.agents, project, &mut session)
                    .await?;
        }
        Ok(ArchitectureOutcome {
            session,
            question,
            message,
        })
    }
}
