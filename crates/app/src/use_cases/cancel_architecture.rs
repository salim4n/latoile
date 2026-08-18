//! Cancel an active architecture discovery session. Provider cancellation is
//! requested first; if it cannot be confirmed the durable workflow remains
//! active and retryable rather than claiming the process stopped.

use super::UseCaseError;
use crate::store::Store;
use latoile_core::ids::ProjectId;
use latoile_core::ports::{AgentChannel, ArchitectureSessionStore};
use latoile_core::ArchitectureSession;

pub struct CancelArchitecture<A> {
    store: Store,
    agents: A,
}

impl<A: AgentChannel> CancelArchitecture<A> {
    pub fn new(store: Store, agents: A) -> Self {
        Self { store, agents }
    }

    pub async fn execute(&self, project: &ProjectId) -> Result<ArchitectureSession, UseCaseError> {
        let mut session = self
            .store
            .active_for_project(project)
            .await?
            .ok_or(UseCaseError::NotFound("active architecture session"))?;
        self.agents.cancel_architecture(&session.id).await?;
        session.cancel()?;
        ArchitectureSessionStore::save(&self.store, &session).await?;
        Ok(session)
    }
}
