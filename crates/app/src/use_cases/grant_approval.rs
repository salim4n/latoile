//! `GrantApproval` — the owner grants a pending approval. A granted *review*
//! approval also moves its task to `done` (the only way there); spec and
//! permission approvals only record the decision.

use super::UseCaseError;
use crate::review_result::review_payload_is_approvable;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{ApprovalId, TaskId};
use latoile_core::ports::{ApprovalStore, EventLog, RunStore, TaskStore};
use latoile_core::{Approval, ApprovalKind, ApprovalStatus, TaskStatus};

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
        self.execute_with_comment(id, None).await
    }

    pub async fn execute_with_comment(
        &self,
        id: &ApprovalId,
        comment: Option<String>,
    ) -> Result<GrantedApproval, UseCaseError> {
        // Fetch the full audit record: retrying the same decision returns
        // the terminal result without replaying task transitions or events.
        let mut approval = self
            .approvals
            .get(id)
            .await?
            .ok_or(UseCaseError::NotFound("approval"))?;

        if approval.status == ApprovalStatus::Granted {
            let task_completed = if approval.kind == ApprovalKind::Review {
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
                (task.status == TaskStatus::Done).then_some(task.id)
            } else {
                None
            };
            return Ok(GrantedApproval {
                approval,
                task_completed,
            });
        }

        // 3. Domain. `grant` is only valid from pending (guaranteed above);
        // a review approval then drives the task to `done`, and the domain
        // refuses if the task is not in review.
        if approval.kind == ApprovalKind::Review && !review_payload_is_approvable(&approval.payload)
        {
            return Err(latoile_core::DomainError::Invariant(
                "only a trusted and approvable Reviewer V2 verdict can be granted",
            )
            .into());
        }
        approval.grant_with_comment(comment)?;
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

    fn approvable_review_payload(reviewed_run: &latoile_core::RunId) -> String {
        serde_json::json!({
            "schema_version": 2,
            "reviewed_run_id": reviewed_run.as_str(),
            "verdict": "approve",
            "summary": "Le verdict et ses preuves ont passé le gate serveur.",
            "findings": [],
            "suggested_follow_ups": [],
            "visual_evidence": {
                "applicability": "not_applicable",
                "references": []
            },
            "gate": {
                "trusted_v2": true,
                "approvable": true,
                "code": "trusted",
                "message": "Preuves exactes."
            }
        })
        .to_string()
    }

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
            approvable_review_payload(&run.id),
        );
        ApprovalStore::save(&store, &approval).await.unwrap();

        let uc = GrantApproval::new(store.clone(), store.clone(), store.clone(), store.clone());
        let out = uc
            .execute_with_comment(&approval.id, Some("Validé".into()))
            .await
            .unwrap();

        assert_eq!(out.approval.status, ApprovalStatus::Granted);
        assert_eq!(out.task_completed, Some(task.id.clone()));
        assert_eq!(
            TaskStore::get(&store, &task.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Done
        );
        assert!(store.list_pending().await.unwrap().is_empty());

        let retried = uc.execute(&approval.id).await.unwrap();
        assert_eq!(retried.approval.status, ApprovalStatus::Granted);
        assert_eq!(retried.approval.decision_comment.as_deref(), Some("Validé"));
        assert_eq!(
            store
                .events_since(0)
                .await
                .unwrap()
                .iter()
                .filter(|(_, event)| event.kind == EventKind::ApprovalGranted)
                .count(),
            1
        );
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
            approvable_review_payload(&run),
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

    #[tokio::test]
    async fn legacy_v1_and_non_approvable_v2_reviews_cannot_be_granted() {
        for payload in [
            serde_json::json!({
                "schema_version": 1,
                "verdict": "approve",
                "summary": "Legacy",
                "findings": [],
                "suggested_follow_ups": []
            })
            .to_string(),
            serde_json::json!({
                "schema_version": 2,
                "reviewed_run_id": "r1",
                "verdict": "changes_requested",
                "summary": "Capture bloquante.",
                "findings": [{
                    "severity": "blocking",
                    "text": "Écart visuel.",
                    "location": "visual:home"
                }],
                "suggested_follow_ups": [],
                "visual_evidence": {
                    "applicability": "required",
                    "references": []
                },
                "gate": {
                    "trusted_v2": false,
                    "approvable": false,
                    "code": "visual_evidence_blocking",
                    "message": "Corriger le rendu."
                }
            })
            .to_string(),
        ] {
            let (store, run) = test_fixtures::store_with_run().await;
            let approval = Approval::new(
                ApprovalId::new("untrusted").unwrap(),
                run,
                ApprovalKind::Review,
                payload,
            );
            ApprovalStore::save(&store, &approval).await.unwrap();
            let uc = GrantApproval::new(store.clone(), store.clone(), store.clone(), store.clone());
            assert!(uc.execute(&approval.id).await.is_err());
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
}
