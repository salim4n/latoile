//! `RejectApproval` — the owner refuses a pending approval. The mirror of
//! `GrantApproval`: nothing else moves (a rejected review leaves the task in
//! review; re-planning is the Manager's job).

use super::UseCaseError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::ApprovalId;
use latoile_core::ports::{ApprovalStore, EventLog, RunStore, TaskStore};
use latoile_core::Approval;

pub struct RejectApproval<A, R, T, E> {
    approvals: A,
    runs: R,
    tasks: T,
    events: E,
}

impl<A: ApprovalStore, R: RunStore, T: TaskStore, E: EventLog> RejectApproval<A, R, T, E> {
    pub fn new(approvals: A, runs: R, tasks: T, events: E) -> Self {
        Self {
            approvals,
            runs,
            tasks,
            events,
        }
    }

    pub async fn execute(&self, id: &ApprovalId) -> Result<Approval, UseCaseError> {
        // 2. Fetch: only a pending approval can be rejected.
        let mut approval = self
            .approvals
            .list_pending()
            .await?
            .into_iter()
            .find(|a| &a.id == id)
            .ok_or(UseCaseError::NotFound("pending approval"))?;

        // 3. Domain.
        approval.reject()?;

        // 4. Persist.
        self.approvals.save(&approval).await?;

        // 5. Journal — the project comes via the run and its task.
        let run = self
            .runs
            .get(&approval.run_id)
            .await?
            .ok_or(UseCaseError::NotFound("run"))?;
        let task = self
            .tasks
            .get(&run.task_id)
            .await?
            .ok_or(UseCaseError::NotFound("task"))?;
        self.events
            .append(&NewEvent {
                project_id: task.project_id,
                kind: EventKind::ApprovalRejected,
                payload: format!("{{\"approval_id\":\"{}\"}}", approval.id),
            })
            .await?;

        // 6. DTO.
        Ok(approval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ids::{ApprovalId as Id, RunId};
    use latoile_core::{ApprovalKind, ApprovalStatus};

    #[tokio::test]
    async fn a_pending_approval_is_rejected_and_journaled() {
        let (store, run) = test_fixtures::store_with_run().await;
        let approval = Approval::new(
            Id::new("a1").unwrap(),
            run,
            ApprovalKind::Review,
            "{}".into(),
        );
        ApprovalStore::save(&store, &approval).await.unwrap();

        let uc = RejectApproval::new(store.clone(), store.clone(), store.clone(), store.clone());
        let rejected = uc.execute(&approval.id).await.unwrap();

        assert_eq!(rejected.status, ApprovalStatus::Rejected);
        assert!(store.list_pending().await.unwrap().is_empty());
        let events = store.since(&test_fixtures::PROJECT, 0).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1.kind, EventKind::ApprovalRejected);
    }

    #[tokio::test]
    async fn an_unknown_or_decided_approval_is_refused() {
        let (store, run) = test_fixtures::store_with_run().await;
        let mut approval = Approval::new(
            Id::new("a1").unwrap(),
            RunId::new(run.as_str()).unwrap(),
            ApprovalKind::Permission,
            "{}".into(),
        );
        approval.grant().unwrap();
        ApprovalStore::save(&store, &approval).await.unwrap();

        let uc = RejectApproval::new(store.clone(), store.clone(), store.clone(), store.clone());
        assert!(uc.execute(&approval.id).await.is_err(), "already decided");
        assert!(uc.execute(&Id::new("ghost").unwrap()).await.is_err(), "unknown");
    }
}
