//! `GrantApproval` — the owner grants a pending approval. A granted *review*
//! approval also moves its task to `done` (the only way there); spec and
//! permission approvals only record the decision.

use super::UseCaseError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{ApprovalId, TaskId};
use latoile_core::ports::{ApprovalStore, EventLog, RunStore, TaskStore};
use latoile_core::{Approval, ApprovalKind};

pub struct GrantedApproval {
    pub approval: Approval,
    /// Set when a review approval closed its task.
    pub task_completed: Option<TaskId>,
}

pub struct GrantApproval<A, R, T, E> {
    approvals: A,
    runs: R,
    tasks: T,
    events: E,
}

impl<A: ApprovalStore, R: RunStore, T: TaskStore, E: EventLog> GrantApproval<A, R, T, E> {
    pub fn new(approvals: A, runs: R, tasks: T, events: E) -> Self {
        Self {
            approvals,
            runs,
            tasks,
            events,
        }
    }

    pub async fn execute(&self, id: &ApprovalId) -> Result<GrantedApproval, UseCaseError> {
        // 2. Fetch: only a pending approval can be granted — the pending
        // list doubles as the fetch (the port exposes no `get`).
        let mut approval = self
            .approvals
            .list_pending()
            .await?
            .into_iter()
            .find(|a| &a.id == id)
            .ok_or(UseCaseError::NotFound("pending approval"))?;

        // 3. Domain. `grant` is only valid from pending (guaranteed above);
        // a review approval then drives the task to `done`, and the domain
        // refuses if the task is not in review.
        approval.grant()?;
        let mut completed = None;
        if approval.kind == ApprovalKind::Review {
            let run = self
                .runs
                .get(&approval.run_id)
                .await?
                .ok_or(UseCaseError::NotFound("run"))?;
            let mut task = self
                .tasks
                .get(&run.task_id)
                .await?
                .ok_or(UseCaseError::NotFound("task"))?;
            task.approve(&approval)?;
            self.tasks.save(&task).await?;
            completed = Some(task.id);
        }

        // 4. Persist the decision.
        self.approvals.save(&approval).await?;

        // 5. Journal.
        self.events
            .append(&NewEvent {
                project_id: self.project_of(&approval).await?,
                kind: EventKind::ApprovalGranted,
                payload: format!("{{\"approval_id\":\"{}\"}}", approval.id),
            })
            .await?;

        // 6. DTO.
        Ok(GrantedApproval {
            approval,
            task_completed: completed,
        })
    }

    /// The project an approval belongs to, via its run and task.
    async fn project_of(
        &self,
        approval: &Approval,
    ) -> Result<latoile_core::ids::ProjectId, UseCaseError> {
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
        Ok(task.project_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::{ApprovalStatus, TaskStatus};

    /// A granted review approval drives its task to `done`.
    #[tokio::test]
    async fn granting_a_review_approval_completes_the_task() {
        let (store, run) = test_fixtures::store_with_run().await;
        // Drive the task to review, the state `approve` expects.
        let mut run = latoile_core::Run::new(
            run,
            latoile_core::ids::TaskId::new("t1").unwrap(),
            latoile_core::RoleId::new("frontend").unwrap(),
            latoile_core::TriggeredBy::Manager,
        );
        run.begin().unwrap();
        RunStore::save(&store, &run).await.unwrap();
        let mut task = TaskStore::get(&store, &run.task_id).await.unwrap().unwrap();
        task.start().unwrap();
        task.submit_for_review().unwrap();
        TaskStore::save(&store, &task).await.unwrap();

        let approval = Approval::new(
            ApprovalId::new("a1").unwrap(),
            run.id.clone(),
            ApprovalKind::Review,
            "{}".into(),
        );
        ApprovalStore::save(&store, &approval).await.unwrap();

        let uc = GrantApproval::new(store.clone(), store.clone(), store.clone(), store.clone());
        let out = uc.execute(&approval.id).await.unwrap();

        assert_eq!(out.approval.status, ApprovalStatus::Granted);
        assert_eq!(out.task_completed, Some(task.id.clone()));
        assert_eq!(
            TaskStore::get(&store, &task.id).await.unwrap().unwrap().status,
            TaskStatus::Done
        );
        assert!(store.list_pending().await.unwrap().is_empty());
    }

    /// Spec and permission approvals do not touch the task.
    #[tokio::test]
    async fn granting_a_permission_approval_only_records_the_decision() {
        let (store, run) = test_fixtures::store_with_run().await;
        let approval = Approval::new(
            ApprovalId::new("a1").unwrap(),
            run,
            ApprovalKind::Permission,
            "{}".into(),
        );
        ApprovalStore::save(&store, &approval).await.unwrap();

        let uc = GrantApproval::new(store.clone(), store.clone(), store.clone(), store.clone());
        let out = uc.execute(&approval.id).await.unwrap();

        assert_eq!(out.approval.status, ApprovalStatus::Granted);
        assert_eq!(out.task_completed, None);
    }

    /// A review approval on a task that is not in review is refused by the
    /// domain — and the decision is not persisted.
    #[tokio::test]
    async fn a_review_approval_cannot_complete_a_task_not_in_review() {
        let (store, run) = test_fixtures::store_with_run().await;
        let approval = Approval::new(
            ApprovalId::new("a1").unwrap(),
            run.clone(),
            ApprovalKind::Review,
            "{}".into(),
        );
        ApprovalStore::save(&store, &approval).await.unwrap();

        let uc = GrantApproval::new(store.clone(), store.clone(), store.clone(), store.clone());
        assert!(uc.execute(&approval.id).await.is_err());

        // Still pending: the grant was rolled back by the refused transition.
        assert_eq!(store.list_pending().await.unwrap().len(), 1);
        assert_eq!(
            RunStore::get(&store, &run).await.unwrap().unwrap().status,
            latoile_core::RunStatus::Starting
        );
    }

    #[tokio::test]
    async fn an_unknown_approval_is_refused() {
        let (store, _) = test_fixtures::store_with_run().await;
        let uc = GrantApproval::new(store.clone(), store.clone(), store.clone(), store);
        assert!(uc
            .execute(&ApprovalId::new("ghost").unwrap())
            .await
            .is_err());
    }
}
