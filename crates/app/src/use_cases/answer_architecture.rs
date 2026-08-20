//! Route one owner answer from the Manager surface back into the live
//! Architect session. The answer is committed before the provider is called;
//! a failed process never eats the owner's decision.

use super::produce_architecture_package::produce_architecture_package;
use super::start_architecture::{
    apply_turn, persist_architect_message, question_for, ArchitectureOutcome,
};
use super::UseCaseError;
use crate::architecture_turn::{
    parse_architecture_turn, ArchitectureTurn, ArchitectureTurnKind,
};
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

        let mut reply = match self
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
        let (validated_session, turn) = match validate_turn(&session, &reply.content) {
            Ok(validated) => validated,
            Err(_) => {
                reply = match self
                    .agents
                    .retry_architecture_contract(project, &session.id, session.phase)
                    .await
                {
                    Ok(reply) => reply,
                    Err(error) => {
                        session.fail(
                            "Architect contract repair could not complete; restart discovery",
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
                        "Architect context or pinned skill provenance changed during contract repair";
                    session.fail(reason)?;
                    ArchitectureSessionStore::save(&self.store, &session).await?;
                    return Err(latoile_core::ports::PortError(reason.into()).into());
                }
                match validate_turn(&session, &reply.content) {
                    Ok(validated) => validated,
                    Err(reason) => {
                        let failure = format!("Architect contract repair failed: {reason}");
                        session.fail(&failure)?;
                        ArchitectureSessionStore::save(&self.store, &session).await?;
                        return Err(latoile_core::ports::PortError(failure).into());
                    }
                }
            }
        };
        session = validated_session;
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

fn validate_turn(
    session: &latoile_core::ArchitectureSession,
    content: &str,
) -> Result<(latoile_core::ArchitectureSession, ArchitectureTurn), String> {
    let turn = parse_architecture_turn(content).map_err(str::to_string)?;
    let mut candidate = session.clone();
    apply_turn(&mut candidate, &turn).map_err(|error| error.to_string())?;
    Ok((candidate, turn))
}
