//! Resolve one live ACP permission request and persist the matching owner
//! decision. The live responder is consumed before the run leaves `blocked`;
//! retries return the terminal audit record without replaying ACP.

use super::UseCaseError;
use latoile_core::error::DomainError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::ApprovalId;
use latoile_core::ports::{AgentChannel, ApprovalStore, EventLog, RunStore, TaskStore};
use latoile_core::{Approval, ApprovalKind, ApprovalStatus, RunStatus};
use serde::Deserialize;

pub struct DecidePermission<A, R, T, E, C> {
    approvals: A,
    runs: R,
    tasks: T,
    events: E,
    agents: C,
}

#[derive(Deserialize)]
struct PermissionPayload {
    request_id: String,
}

impl<A: ApprovalStore, R: RunStore, T: TaskStore, E: EventLog, C: AgentChannel>
    DecidePermission<A, R, T, E, C>
{
    pub fn new(approvals: A, runs: R, tasks: T, events: E, agents: C) -> Self {
        Self {
            approvals,
            runs,
            tasks,
            events,
            agents,
        }
    }

    pub async fn execute(
        &self,
        id: &ApprovalId,
        granted: bool,
        comment: Option<String>,
    ) -> Result<Approval, UseCaseError> {
        let approval = self
            .approvals
            .get(id)
            .await?
            .ok_or(UseCaseError::NotFound("approval"))?;
        if approval.kind != ApprovalKind::Permission {
            return Err(DomainError::Invariant("approval is not a permission request").into());
        }

        let wanted = if granted {
            ApprovalStatus::Granted
        } else {
            ApprovalStatus::Rejected
        };
        if approval.status == wanted {
            return Ok(approval);
        }

        let payload: PermissionPayload = serde_json::from_str(&approval.payload)
            .map_err(|_| DomainError::Invariant("invalid permission approval payload"))?;
        if payload.request_id.trim().is_empty() {
            return Err(DomainError::Invariant("invalid permission approval payload").into());
        }

        let mut decided = approval.clone();
        if granted {
            decided.grant_with_comment(comment)?;
        } else {
            decided.reject_with_comment(comment)?;
        }

        let mut run = self
            .runs
            .get(&approval.run_id)
            .await?
            .ok_or(UseCaseError::NotFound("run"))?;
        if run.status != RunStatus::Blocked {
            return Err(DomainError::Invariant("permission run is not blocked").into());
        }
        let task = self
            .tasks
            .get(&run.task_id)
            .await?
            .ok_or(UseCaseError::NotFound("task"))?;

        // This consumes the in-memory responder. A restart/lost session
        // fails here and leaves the database pending for supervision to
        // close fail-closed.
        self.agents
            .resolve_permission(&run.id, &payload.request_id, granted)
            .await?;
        run.resume()?;
        self.runs.save(&run).await?;
        self.approvals.save(&decided).await?;
        self.events
            .append(&NewEvent {
                project_id: task.project_id,
                kind: if granted {
                    EventKind::ApprovalGranted
                } else {
                    EventKind::ApprovalRejected
                },
                payload: serde_json::json!({
                    "approval_id": decided.id.as_str(),
                    "permission_request_id": payload.request_id,
                })
                .to_string(),
            })
            .await?;
        Ok(decided)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ids::{ProjectId, RunId};
    use latoile_core::ports::{ManagerReply, PortError, PortResult};
    use latoile_core::{ApprovalId, Run};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FakeAgents {
        pending: Arc<Mutex<Option<String>>>,
        decisions: Arc<Mutex<Vec<bool>>>,
    }

    impl FakeAgents {
        fn live(request_id: &str) -> Self {
            Self {
                pending: Arc::new(Mutex::new(Some(request_id.into()))),
                decisions: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn lost() -> Self {
            Self::live("")
        }
    }

    impl AgentChannel for FakeAgents {
        async fn tell_manager(&self, _: &ProjectId, _: &str) -> PortResult<ManagerReply> {
            unimplemented!()
        }

        async fn start_run(&self, _: &Run, _: &str) -> PortResult<String> {
            unimplemented!()
        }

        async fn resolve_permission(
            &self,
            _: &RunId,
            request_id: &str,
            granted: bool,
        ) -> PortResult<()> {
            let mut pending = self.pending.lock().unwrap();
            if pending.as_deref() != Some(request_id) {
                return Err(PortError("lost permission session".into()));
            }
            pending.take();
            self.decisions.lock().unwrap().push(granted);
            Ok(())
        }

        async fn cancel_run(&self, _: &RunId) -> PortResult<()> {
            Ok(())
        }
    }

    async fn fixture() -> (crate::store::Store, Approval) {
        let (store, run_id) = test_fixtures::store_with_run().await;
        let mut run = RunStore::get(&store, &run_id).await.unwrap().unwrap();
        run.begin().unwrap();
        run.block().unwrap();
        RunStore::save(&store, &run).await.unwrap();
        let approval = Approval::new(
            ApprovalId::new("permission-perm-1").unwrap(),
            run_id,
            ApprovalKind::Permission,
            serde_json::json!({
                "schema_version": 1,
                "request_id": "perm-1",
                "summary": "Execute a command inside the project workspace",
            })
            .to_string(),
        );
        ApprovalStore::save(&store, &approval).await.unwrap();
        (store, approval)
    }

    #[tokio::test]
    async fn grant_resumes_the_exact_request_once() {
        let (store, approval) = fixture().await;
        let agents = FakeAgents::live("perm-1");
        let uc = DecidePermission::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            agents.clone(),
        );
        let granted = uc.execute(&approval.id, true, None).await.unwrap();
        assert_eq!(granted.status, ApprovalStatus::Granted);
        assert_eq!(agents.decisions.lock().unwrap().as_slice(), [true]);
        assert_eq!(
            RunStore::get(&store, &approval.run_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            RunStatus::Running
        );

        let retried = uc.execute(&approval.id, true, None).await.unwrap();
        assert_eq!(retried.status, ApprovalStatus::Granted);
        assert_eq!(agents.decisions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rejection_is_forwarded_and_persisted() {
        let (store, approval) = fixture().await;
        let agents = FakeAgents::live("perm-1");
        let uc = DecidePermission::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            agents.clone(),
        );
        let rejected = uc
            .execute(&approval.id, false, Some("Pas nécessaire".into()))
            .await
            .unwrap();
        assert_eq!(rejected.status, ApprovalStatus::Rejected);
        assert_eq!(rejected.decision_comment.as_deref(), Some("Pas nécessaire"));
        assert_eq!(agents.decisions.lock().unwrap().as_slice(), [false]);
    }

    #[tokio::test]
    async fn a_lost_session_cannot_be_granted() {
        let (store, approval) = fixture().await;
        let uc = DecidePermission::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            FakeAgents::lost(),
        );
        assert!(uc.execute(&approval.id, true, None).await.is_err());
        assert_eq!(
            ApprovalStore::get(&store, &approval.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ApprovalStatus::Pending
        );
    }
}
