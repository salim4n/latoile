//! `RejectApproval` — the owner requests changes. A review rejection records
//! the owner's comment and starts one corrective executor run on the same
//! task; retries return that same decision without spawning again.

use super::UseCaseError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{ApprovalId, RunId};
use latoile_core::ports::{AgentChannel, ApprovalStore, EventLog, RunStore, TaskStore};
use latoile_core::{Approval, ApprovalKind, ApprovalStatus, Run, TriggeredBy};

pub struct RejectApproval<A, R, T, E, C> {
    approvals: A,
    runs: R,
    tasks: T,
    events: E,
    agents: C,
}

impl<A: ApprovalStore, R: RunStore, T: TaskStore, E: EventLog, C: AgentChannel>
    RejectApproval<A, R, T, E, C>
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

    pub async fn execute(&self, id: &ApprovalId) -> Result<Approval, UseCaseError> {
        self.execute_with_comment(id, None).await
    }

    pub async fn execute_with_comment(
        &self,
        id: &ApprovalId,
        comment: Option<String>,
    ) -> Result<Approval, UseCaseError> {
        let mut approval = self
            .approvals
            .get(id)
            .await?
            .ok_or(UseCaseError::NotFound("approval"))?;

        if approval.status == ApprovalStatus::Rejected {
            return Ok(approval);
        }

        approval.reject_with_comment(comment)?;

        let reviewed_run = self
            .runs
            .get(&approval.run_id)
            .await?
            .ok_or(UseCaseError::NotFound("run"))?;
        let mut task = self
            .tasks
            .get(&reviewed_run.task_id)
            .await?
            .ok_or(UseCaseError::NotFound("task"))?;
        let mut corrective_run = None;

        if approval.kind == ApprovalKind::Review {
            task.request_changes()?;

            let previous_runs = self.runs.list_for_task(&task.id).await?;
            let executor = previous_runs
                .iter()
                .rev()
                .find(|run| run.role_id.as_str() != "reviewer")
                .ok_or(UseCaseError::NotFound("executor run"))?;
            let prompt = correction_prompt(
                &task,
                &approval,
                executor.summary.as_deref().unwrap_or("(none)"),
                executor.artifacts.as_deref().unwrap_or("{}"),
            );

            task.requeue()?;
            task.start()?;
            let mut run = Run::new(
                RunId::new(ulid::Ulid::new().to_string())?,
                task.id.clone(),
                executor.role_id.clone(),
                TriggeredBy::User,
            );
            let session = self
                .agents
                .start_run(&task.project_id, &run, &prompt)
                .await?;
            run.acp_session_id = Some(session);
            run.begin()?;
            approval.attach_corrective_run(run.id.clone())?;
            corrective_run = Some(run);
        }

        // Persist only after the corrective process started successfully. A
        // spawn failure leaves the review pending and safe to retry.
        self.tasks.save(&task).await?;
        if let Some(run) = &corrective_run {
            self.runs.save(run).await?;
        }
        self.approvals.save(&approval).await?;

        self.events
            .append(&NewEvent {
                project_id: task.project_id.clone(),
                kind: EventKind::ApprovalRejected,
                payload: serde_json::json!({
                    "approval_id": approval.id.as_str(),
                    "corrective_run_id": approval
                        .corrective_run_id
                        .as_ref()
                        .map(|run| run.as_str()),
                    "has_comment": approval.decision_comment.is_some(),
                })
                .to_string(),
            })
            .await?;
        if let Some(run) = corrective_run {
            self.events
                .append(&NewEvent {
                    project_id: task.project_id,
                    kind: EventKind::RunStarted,
                    payload: serde_json::json!({
                        "task_id": task.id.as_str(),
                        "run_id": run.id.as_str(),
                        "corrective": true,
                    })
                    .to_string(),
                })
                .await?;
        }

        Ok(approval)
    }
}

fn correction_prompt(
    task: &latoile_core::Task,
    approval: &Approval,
    previous_summary: &str,
    previous_artifacts: &str,
) -> String {
    format!(
        "Correct the implementation for task `{}`.\n\nTASK\n{}\n\nREVIEWER RESULT\n{}\n\nOWNER COMMENT\n{}\n\nPREVIOUS EXECUTION\nsummary: {}\nartifacts: {}\n\nAddress every blocking finding and the owner's comment. Keep the approved specification and visual contract unchanged. Run the relevant tests and report the result.",
        task.title,
        task.description,
        truncate(&approval.payload, 32 * 1024),
        approval.decision_comment.as_deref().unwrap_or("(none)"),
        truncate(previous_summary, 8 * 1024),
        truncate(previous_artifacts, 16 * 1024),
    )
}

fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut boundary = limit;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}…", &value[..boundary])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ids::{ApprovalId as Id, RoleId, TaskId};
    use latoile_core::ports::{ManagerReply, PortError};
    use latoile_core::{ApprovalKind, ApprovalStatus, RunStatus, TaskStatus};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct FakeAgents {
        prompts: Arc<Mutex<Vec<String>>>,
    }

    impl AgentChannel for FakeAgents {
        async fn tell_manager(
            &self,
            _: &latoile_core::ids::ProjectId,
            _: &str,
        ) -> Result<ManagerReply, PortError> {
            unimplemented!()
        }

        async fn start_run(
            &self,
            _: &latoile_core::ids::ProjectId,
            _: &Run,
            prompt: &str,
        ) -> Result<String, PortError> {
            self.prompts.lock().unwrap().push(prompt.into());
            Ok("acp-correction".into())
        }

        async fn cancel_run(&self, _: &RunId) -> Result<(), PortError> {
            Ok(())
        }
    }

    async fn review_fixture() -> (crate::store::Store, Approval) {
        let (store, task_id) = test_fixtures::store_with_task().await;
        let mut task = TaskStore::get(&store, &task_id).await.unwrap().unwrap();
        task.start().unwrap();

        let mut executor = Run::new(
            RunId::new("executor-1").unwrap(),
            task.id.clone(),
            RoleId::new("frontend").unwrap(),
            TriggeredBy::Manager,
        );
        executor.begin().unwrap();
        executor.finish("Implemented the login page").unwrap();
        executor
            .attach_evidence(None, None, r#"{"changed_files":["Login.tsx"]}"#.into())
            .unwrap();
        RunStore::save(&store, &executor).await.unwrap();

        task.submit_for_review().unwrap();
        TaskStore::save(&store, &task).await.unwrap();
        let mut reviewer = Run::new(
            RunId::new("reviewer-1").unwrap(),
            task.id.clone(),
            RoleId::new("reviewer").unwrap(),
            TriggeredBy::Manager,
        );
        reviewer.begin().unwrap();
        reviewer.finish("review complete").unwrap();
        RunStore::save(&store, &reviewer).await.unwrap();

        let approval = Approval::new(
            Id::new("a1").unwrap(),
            reviewer.id,
            ApprovalKind::Review,
            serde_json::json!({
                "verdict": "changes_requested",
                "findings": [{
                    "severity": "blocking",
                    "text": "Le focus clavier est absent.",
                    "location": "web/src/Login.tsx:42"
                }]
            })
            .to_string(),
        );
        ApprovalStore::save(&store, &approval).await.unwrap();
        (store, approval)
    }

    #[tokio::test]
    async fn a_review_rejection_persists_comment_and_starts_one_correction() {
        let (store, approval) = review_fixture().await;
        let agents = FakeAgents::default();
        let uc = RejectApproval::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            agents.clone(),
        );
        assert!(
            uc.execute(&approval.id).await.is_err(),
            "comment is required"
        );
        let rejected = uc
            .execute_with_comment(
                &approval.id,
                Some("Corriger le focus puis ajouter le test clavier.".into()),
            )
            .await
            .unwrap();

        assert_eq!(rejected.status, ApprovalStatus::Rejected);
        assert_eq!(
            rejected.decision_comment.as_deref(),
            Some("Corriger le focus puis ajouter le test clavier.")
        );
        let corrective = rejected.corrective_run_id.clone().unwrap();
        assert!(store.list_pending().await.unwrap().is_empty());
        let task = TaskStore::get(&store, &TaskId::new("t1").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.status, TaskStatus::InProgress);
        let run = RunStore::get(&store, &corrective).await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Running);
        assert_eq!(run.role_id.as_str(), "frontend");
        let prompt = agents.prompts.lock().unwrap()[0].clone();
        assert!(prompt.contains("Le focus clavier est absent"));
        assert!(prompt.contains("Corriger le focus puis ajouter le test clavier"));

        let events = store.since(&test_fixtures::PROJECT, 0).await.unwrap();
        assert_eq!(
            events
                .iter()
                .map(|(_, event)| event.kind)
                .collect::<Vec<_>>(),
            [EventKind::ApprovalRejected, EventKind::RunStarted]
        );

        // A retry returns the same audit record and never starts another run.
        let retried = uc
            .execute_with_comment(&approval.id, Some("different retry text".into()))
            .await
            .unwrap();
        assert_eq!(retried.corrective_run_id.as_ref(), Some(&corrective));
        assert_eq!(agents.prompts.lock().unwrap().len(), 1);
        assert_eq!(
            RunStore::list_for_task(&store, &task.id)
                .await
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            store.since(&test_fixtures::PROJECT, 0).await.unwrap().len(),
            2
        );
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

        let uc = RejectApproval::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            FakeAgents::default(),
        );
        assert!(uc.execute(&approval.id).await.is_err(), "already decided");
        assert!(
            uc.execute(&Id::new("ghost").unwrap()).await.is_err(),
            "unknown"
        );
    }
}
