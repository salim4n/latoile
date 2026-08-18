//! Supervision tests: pure plans and store-backed applies.

use super::*;
use crate::store::test_fixtures;
use latoile_core::ids::{RoleId, TaskId};
use latoile_core::ports::PermissionRequest;
use latoile_core::{Run, TriggeredBy};

/// A run mid-flight (Running) on the fixture task, which is InProgress.
async fn store_with_running_run() -> (Store, Run, Task) {
    let (store, task_id) = test_fixtures::store_with_task().await;
    let mut task = TaskStore::get(&store, &task_id).await.unwrap().unwrap();
    task.start().unwrap();
    TaskStore::save(&store, &task).await.unwrap();
    let mut run = Run::new(
        RunId::new("r9").unwrap(),
        task.id.clone(),
        RoleId::new("frontend").unwrap(),
        TriggeredBy::Manager,
    );
    run.begin().unwrap();
    RunStore::save(&store, &run).await.unwrap();
    (store, run, task)
}

fn permission() -> PermissionRequest {
    PermissionRequest {
        id: "perm-1".into(),
        summary: "Execute a command inside the project workspace".into(),
    }
}

#[tokio::test]
async fn an_ask_blocks_the_run_and_creates_one_sanitized_inbox_item() {
    let (store, run, _) = store_with_running_run().await;
    apply(
        &store,
        &run.id,
        &Observed::PermissionRequested(permission()),
    )
    .await
    .unwrap();

    assert_eq!(
        RunStore::get(&store, &run.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Blocked
    );
    let pending = ApprovalStore::list_pending(&store).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, ApprovalKind::Permission);
    let payload: serde_json::Value = serde_json::from_str(&pending[0].payload).unwrap();
    assert_eq!(payload["request_id"], "perm-1");
    assert_eq!(
        payload["summary"],
        "Execute a command inside the project workspace"
    );
    let kinds = store
        .events_since(0)
        .await
        .unwrap()
        .into_iter()
        .map(|(_, event)| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(kinds, [EventKind::RunBlocked, EventKind::ApprovalRequested]);

    // Polling the same parked callback is an upsert, not another card/event.
    apply(
        &store,
        &run.id,
        &Observed::PermissionRequested(permission()),
    )
    .await
    .unwrap();
    assert_eq!(ApprovalStore::list_pending(&store).await.unwrap().len(), 1);
    assert_eq!(store.events_since(0).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_permission_timeout_refuses_the_card_and_resumes_the_run() {
    let (store, run, _) = store_with_running_run().await;
    apply(
        &store,
        &run.id,
        &Observed::PermissionRequested(permission()),
    )
    .await
    .unwrap();
    apply(&store, &run.id, &Observed::PermissionExpired(permission()))
        .await
        .unwrap();

    assert_eq!(
        RunStore::get(&store, &run.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Running
    );
    assert!(ApprovalStore::list_pending(&store)
        .await
        .unwrap()
        .is_empty());
    let decided = ApprovalStore::get(&store, &ApprovalId::new("permission-perm-1").unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decided.status, latoile_core::ApprovalStatus::Rejected);
    assert_eq!(
        decided.decision_comment.as_deref(),
        Some("permission request timed out")
    );
}

#[tokio::test]
async fn cancellation_and_restart_close_pending_permission_cards() {
    for (observed, expected) in [
        (Observed::Cancelled, RunStatus::Cancelled),
        (Observed::Lost, RunStatus::Error),
    ] {
        let (store, run, _) = store_with_running_run().await;
        apply(
            &store,
            &run.id,
            &Observed::PermissionRequested(permission()),
        )
        .await
        .unwrap();
        apply(&store, &run.id, &observed).await.unwrap();

        assert_eq!(
            RunStore::get(&store, &run.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            expected
        );
        assert!(ApprovalStore::list_pending(&store)
            .await
            .unwrap()
            .is_empty());
        let decided = ApprovalStore::get(&store, &ApprovalId::new("permission-perm-1").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decided.status, latoile_core::ApprovalStatus::Rejected);
    }
}

#[test]
fn a_running_run_plans_nothing() {
    let run = Run::new(
        RunId::new("r1").unwrap(),
        TaskId::new("t1").unwrap(),
        RoleId::new("backend").unwrap(),
        TriggeredBy::User,
    );
    let task = test_fixtures_task();
    assert!(plan(&run, &task, &Observed::Running).is_empty());
}

fn test_fixtures_task() -> Task {
    Task::new(
        TaskId::new("t1").unwrap(),
        test_fixtures::PROJECT.clone(),
        RoleId::new("backend").unwrap(),
        "T",
        "d",
        0,
    )
    .unwrap()
}

#[test]
fn a_blocked_run_is_resumed_before_it_finishes() {
    let mut run = Run::new(
        RunId::new("r1").unwrap(),
        TaskId::new("t1").unwrap(),
        RoleId::new("backend").unwrap(),
        TriggeredBy::User,
    );
    run.begin().unwrap();
    run.block().unwrap();
    let mut task = test_fixtures_task();
    task.bind_spec(latoile_core::ids::SpecVersionId::new("s1").unwrap());
    task.start().unwrap();

    let steps = plan(&run, &task, &Observed::finished("done"));
    // ResumeRun must precede FinishRun or the domain refuses.
    assert_eq!(steps[0], Step::ResumeRun);
    assert_eq!(
        steps[1],
        Step::FinishRun {
            summary: "done".into(),
            base_sha: None,
            head_sha: None,
            artifacts: "{}".into(),
        }
    );
    assert!(steps.contains(&Step::SubmitForReview));
    assert!(steps.contains(&Step::DispatchReviewer));
    assert!(!steps
        .iter()
        .any(|step| matches!(step, Step::RequestReviewApproval { .. })));
}

#[test]
fn a_task_already_past_in_progress_gets_no_second_review() {
    let mut run = Run::new(
        RunId::new("r1").unwrap(),
        TaskId::new("t1").unwrap(),
        RoleId::new("backend").unwrap(),
        TriggeredBy::User,
    );
    run.begin().unwrap();
    let mut task = test_fixtures_task();
    task.bind_spec(latoile_core::ids::SpecVersionId::new("s1").unwrap());
    task.start().unwrap();
    task.submit_for_review().unwrap(); // already in review

    let steps = plan(&run, &task, &Observed::finished("s"));
    assert!(!steps.contains(&Step::SubmitForReview));
    assert!(!steps.contains(&Step::DispatchReviewer));
    assert!(!steps
        .iter()
        .any(|step| matches!(step, Step::RequestReviewApproval { .. })));
    assert!(steps
        .iter()
        .any(|s| matches!(s, Step::Journal(EventKind::RunFinished, _))));
}

#[tokio::test]
async fn a_finished_executor_enters_review_without_requesting_human_approval() {
    let (store, run, _) = store_with_running_run().await;
    let applied = apply(&store, &run.id, &Observed::finished("endpoint implemented"))
        .await
        .unwrap();

    assert!(applied.review_approval.is_none());
    assert!(applied.reviewer_dispatch_requested);
    let run = RunStore::get(&store, &run.id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Finished);
    let task = TaskStore::get(&store, &run.task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);

    assert!(ApprovalStore::list_pending(&store)
        .await
        .unwrap()
        .is_empty());

    let kinds: Vec<_> = store
        .events_since(0)
        .await
        .unwrap()
        .iter()
        .map(|(_, e)| e.kind)
        .collect();
    assert_eq!(kinds, [EventKind::RunFinished]);

    // A second tick is a no-op.
    let again = apply(&store, &run.id, &Observed::finished("endpoint implemented"))
        .await
        .unwrap();
    assert_eq!(again.steps, 0);
    assert!(!again.reviewer_dispatch_requested);
}

#[tokio::test]
async fn a_terminal_reviewer_creates_one_validated_human_approval() {
    use latoile_core::ports::{ManagerReply, PortError};
    struct FakeAgents;
    impl latoile_core::ports::AgentChannel for FakeAgents {
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
            _: &str,
        ) -> Result<String, PortError> {
            Ok("acp-review".into())
        }
        async fn cancel_run(&self, _: &RunId) -> Result<(), PortError> {
            Ok(())
        }
    }

    let (store, executor, _) = store_with_running_run().await;
    apply(
        &store,
        &executor.id,
        &Observed::finished("endpoint implemented"),
    )
    .await
    .unwrap();
    assert!(ApprovalStore::list_pending(&store)
        .await
        .unwrap()
        .is_empty());

    let reviewer = start_review(
        &store,
        &FakeAgents,
        &executor.task_id,
        &executor.id,
        "context",
    )
    .await
    .unwrap();
    let output = serde_json::json!({
        "schema_version": 1,
        "verdict": "approve",
        "summary": "Le changement respecte la tâche et la spec.",
        "findings": [],
        "suggested_follow_ups": []
    })
    .to_string();
    let applied = apply(&store, &reviewer.id, &Observed::finished(output))
        .await
        .unwrap();

    assert!(!applied.reviewer_dispatch_requested);
    let expected_approval_id = format!("review-{}", reviewer.id.as_str());
    assert_eq!(
        applied.review_approval.as_ref().map(|id| id.as_str()),
        Some(expected_approval_id.as_str())
    );
    let pending = ApprovalStore::list_pending(&store).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].run_id, reviewer.id);
    assert_eq!(pending[0].kind, ApprovalKind::Review);
    let payload: serde_json::Value = serde_json::from_str(&pending[0].payload).unwrap();
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["verdict"], "approve");

    let again = apply(&store, &reviewer.id, &Observed::finished("ignored"))
        .await
        .unwrap();
    assert_eq!(again.steps, 0);
    assert_eq!(ApprovalStore::list_pending(&store).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_reviewer_creates_an_actionable_fallback_approval() {
    let (store, executor, _) = store_with_running_run().await;
    apply(&store, &executor.id, &Observed::finished("done"))
        .await
        .unwrap();

    let mut reviewer = Run::new(
        RunId::new("reviewer-failed").unwrap(),
        executor.task_id,
        RoleId::new("reviewer").unwrap(),
        TriggeredBy::Manager,
    );
    reviewer.begin().unwrap();
    RunStore::save(&store, &reviewer).await.unwrap();
    apply(
        &store,
        &reviewer.id,
        &Observed::Failed {
            reason: "provider unavailable".into(),
        },
    )
    .await
    .unwrap();

    let pending = ApprovalStore::list_pending(&store).await.unwrap();
    assert_eq!(pending.len(), 1);
    let payload: serde_json::Value = serde_json::from_str(&pending[0].payload).unwrap();
    assert_eq!(payload["verdict"], "changes_requested");
    assert!(payload["summary"]
        .as_str()
        .unwrap()
        .contains("provider unavailable"));
}

#[tokio::test]
async fn a_failed_run_is_journaled_and_the_task_returns_to_ready() {
    let (store, run, _) = store_with_running_run().await;
    apply(
        &store,
        &run.id,
        &Observed::Failed {
            reason: "agent crashed".into(),
        },
    )
    .await
    .unwrap();

    let run = RunStore::get(&store, &run.id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Error);
    // Task::fail_run: the board gets the task back, spec still bound.
    let task = TaskStore::get(&store, &run.task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Ready);
    assert!(task.spec_version_id.is_some());
    assert!(ApprovalStore::list_pending(&store)
        .await
        .unwrap()
        .is_empty());
    let events = store.events_since(0).await.unwrap();
    let kinds: Vec<_> = events.iter().map(|(_, e)| e.kind).collect();
    assert_eq!(kinds, [EventKind::RunFinished, EventKind::TaskReady]);
    assert!(events[0].1.payload.contains("agent crashed"));
}

#[tokio::test]
async fn a_lost_run_is_failed_like_a_crash() {
    let (store, run, _) = store_with_running_run().await;
    apply(&store, &run.id, &Observed::Lost).await.unwrap();
    let run = RunStore::get(&store, &run.id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Error);
    let events = store.events_since(0).await.unwrap();
    assert!(events[0].1.payload.contains("server restart"));
}

#[tokio::test]
async fn a_cancelled_run_is_journaled_without_a_review() {
    let (store, run, _) = store_with_running_run().await;
    apply(&store, &run.id, &Observed::Cancelled).await.unwrap();
    let run = RunStore::get(&store, &run.id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Cancelled);
    assert!(ApprovalStore::list_pending(&store)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn the_reviewer_is_dispatched_on_a_task_in_review() {
    use latoile_core::ports::ManagerReply;
    use latoile_core::ports::PortError;
    struct FakeAgents;
    impl latoile_core::ports::AgentChannel for FakeAgents {
        async fn tell_manager(
            &self,
            _: &latoile_core::ids::ProjectId,
            _: &str,
        ) -> Result<ManagerReply, PortError> {
            unimplemented!()
        }
        async fn start_run(
            &self,
            _project: &latoile_core::ids::ProjectId,
            _r: &Run,
            prompt: &str,
        ) -> Result<String, PortError> {
            assert!(prompt.contains("endpoint implemented"), "{prompt}");
            Ok("acp-review".into())
        }
        async fn cancel_run(&self, _: &RunId) -> Result<(), PortError> {
            Ok(())
        }
    }

    let (store, run, _) = store_with_running_run().await;
    apply(&store, &run.id, &Observed::finished("endpoint implemented"))
        .await
        .unwrap();

    let review = start_review(
        &store,
        &FakeAgents,
        &run.task_id,
        &run.id,
        "endpoint implemented",
    )
    .await
    .unwrap();
    assert_eq!(review.role_id.as_str(), "reviewer");
    assert_eq!(review.status, RunStatus::Running);
    // The task itself stays in review — the human decides.
    let task = TaskStore::get(&store, &run.task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert!(store
        .events_since(0)
        .await
        .unwrap()
        .iter()
        .any(|(_, e)| e.kind == EventKind::RunStarted));
}

#[tokio::test]
async fn the_reviewer_is_refused_on_a_task_not_in_review() {
    struct NoAgents;
    impl latoile_core::ports::AgentChannel for NoAgents {
        async fn tell_manager(
            &self,
            _: &latoile_core::ids::ProjectId,
            _: &str,
        ) -> Result<latoile_core::ports::ManagerReply, latoile_core::ports::PortError> {
            unimplemented!()
        }
        async fn start_run(
            &self,
            _project: &latoile_core::ids::ProjectId,
            _r: &Run,
            _p: &str,
        ) -> Result<String, latoile_core::ports::PortError> {
            panic!("must never be called")
        }
        async fn cancel_run(&self, _: &RunId) -> Result<(), latoile_core::ports::PortError> {
            Ok(())
        }
    }

    // Task still in progress: no review run may start on it.
    let (store, run, _) = store_with_running_run().await;
    assert!(start_review(&store, &NoAgents, &run.task_id, &run.id, "")
        .await
        .is_err());
}

#[tokio::test]
async fn a_reviewer_spawn_failure_is_visible_as_a_fallback_approval() {
    struct BrokenAgents;
    impl latoile_core::ports::AgentChannel for BrokenAgents {
        async fn tell_manager(
            &self,
            _: &latoile_core::ids::ProjectId,
            _: &str,
        ) -> Result<latoile_core::ports::ManagerReply, latoile_core::ports::PortError> {
            unimplemented!()
        }
        async fn start_run(
            &self,
            _project: &latoile_core::ids::ProjectId,
            _: &Run,
            _: &str,
        ) -> Result<String, latoile_core::ports::PortError> {
            Err(latoile_core::ports::PortError("binary missing".into()))
        }
        async fn cancel_run(&self, _: &RunId) -> Result<(), latoile_core::ports::PortError> {
            Ok(())
        }
    }

    let (store, executor, _) = store_with_running_run().await;
    apply(&store, &executor.id, &Observed::finished("done"))
        .await
        .unwrap();
    let reviewer = start_review(
        &store,
        &BrokenAgents,
        &executor.task_id,
        &executor.id,
        "context",
    )
    .await
    .unwrap();

    assert_eq!(reviewer.status, RunStatus::Error);
    let pending = ApprovalStore::list_pending(&store).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].run_id, reviewer.id);
    assert!(pending[0].payload.contains("binary missing"));
    let kinds: Vec<_> = store
        .events_since(0)
        .await
        .unwrap()
        .iter()
        .map(|(_, event)| event.kind)
        .collect();
    assert!(kinds.ends_with(&[EventKind::RunFinished, EventKind::ApprovalRequested]));
}
