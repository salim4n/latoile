//! Supervision driver tests: scripted channel states, real store, short
//! poll interval. Covers finish → review flow, failure journaling, and the
//! restart-lost case.

use super::*;
use crate::driver;
use latoile_agents::{ChangedFileEvidence, CommitEvidence, RunOutcome, RunReport, RunState};
use latoile_core::event::EventKind;
use latoile_core::ids::{PreviewId, RunId, SpecVersionId, TaskId};
use latoile_core::ports::PermissionRequest;
use latoile_core::ports::{ApprovalStore, PreviewStore, RunStore, SpecStore, TaskStore};
use latoile_core::{
    Approval, ApprovalId, ApprovalKind, ApprovalStatus, Preview, RoleId, Run, RunStatus,
    SpecVersion, Task, TaskStatus, TriggeredBy,
};
use std::time::Duration;

/// A running run on an in-progress task, straight into the store.
async fn seed_running_run(store: &Store, project: &str, run_id: &str) -> RunId {
    let mut spec = SpecVersion::new(
        SpecVersionId::new("s1").unwrap(),
        ProjectId::new(project).unwrap(),
        1,
        "design/",
        None,
    )
    .unwrap();
    spec.approve().unwrap();
    SpecStore::save(store, &spec).await.unwrap();

    let mut task = Task::new(
        TaskId::new(format!("t-{run_id}")).unwrap(),
        ProjectId::new(project).unwrap(),
        RoleId::new("frontend").unwrap(),
        "Page de connexion",
        "Formulaire",
        0,
    )
    .unwrap();
    task.bind_spec(spec.id.clone());
    task.start().unwrap();
    TaskStore::save(store, &task).await.unwrap();

    let mut run = Run::new(
        RunId::new(run_id).unwrap(),
        task.id,
        RoleId::new("frontend").unwrap(),
        TriggeredBy::Manager,
    );
    run.begin().unwrap();
    RunStore::save(store, &run).await.unwrap();
    run.id
}

async fn run_status(store: &Store, run: &RunId) -> RunStatus {
    RunStore::get(store, run).await.unwrap().unwrap().status
}

/// Poll the store until the run reaches a terminal status.
async fn wait_terminal(store: &Store, run: &RunId) -> RunStatus {
    for _ in 0..100 {
        let status = run_status(store, run).await;
        if !status.is_active() {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("run never left an active status");
}

async fn wait_blocked_with_approval(store: &Store, run: &RunId) -> Approval {
    for _ in 0..100 {
        if run_status(store, run).await == RunStatus::Blocked {
            if let Some(approval) = store.list_pending().await.unwrap().into_iter().next() {
                return approval;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("permission request never reached the Inbox");
}

#[tokio::test]
async fn permission_decisions_block_and_resume_the_exact_http_request_once() {
    for granted in [true, false] {
        let (state, store, agents) = state().await;
        let app = router(state.clone());
        let project = create_project(&app).await;
        let run = seed_running_run(&store, &project, "r-permission").await;
        let request_id = "perm-http";
        agents
            .live_permissions
            .lock()
            .unwrap()
            .insert(run.as_str().to_string(), request_id.into());
        agents.run_states.lock().unwrap().insert(
            run.as_str().to_string(),
            RunState::Blocked(PermissionRequest {
                id: request_id.into(),
                summary: "Execute a command inside the project workspace".into(),
            }),
        );

        let handle = driver::spawn_every(state, Duration::from_millis(20));
        let approval = wait_blocked_with_approval(&store, &run).await;
        assert_eq!(approval.kind, ApprovalKind::Permission);
        let payload: serde_json::Value = serde_json::from_str(&approval.payload).unwrap();
        assert_eq!(payload["request_id"], request_id);
        assert_eq!(
            payload["summary"],
            "Execute a command inside the project workspace"
        );

        let response = app
            .clone()
            .oneshot(authed(request(
                "POST",
                &format!("/api/approvals/{}", approval.id.as_str()),
                Some(serde_json::json!({
                    "granted": granted,
                    "comment": if granted { "Autorisé une fois" } else { "Refusé" },
                })),
            )))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let decided = body_json(response).await;
        assert_eq!(
            decided["status"],
            if granted { "granted" } else { "rejected" }
        );
        assert_eq!(run_status(&store, &run).await, RunStatus::Running);
        assert_eq!(
            agents.permission_decisions.lock().unwrap().as_slice(),
            [(run.as_str().to_string(), request_id.into(), granted)]
        );

        let retry = app
            .clone()
            .oneshot(authed(request(
                "POST",
                &format!("/api/approvals/{}", approval.id.as_str()),
                Some(serde_json::json!({"granted": granted})),
            )))
            .await
            .unwrap();
        assert_eq!(retry.status(), StatusCode::OK);
        assert_eq!(agents.permission_decisions.lock().unwrap().len(), 1);
        handle.abort();
    }
}

#[tokio::test]
async fn a_restart_rejects_an_orphan_permission_and_fails_the_run() {
    let (state, store, _agents) = state().await;
    let app = router(state.clone());
    let project = create_project(&app).await;
    let run_id = seed_running_run(&store, &project, "r-orphan").await;
    let mut run = RunStore::get(&store, &run_id).await.unwrap().unwrap();
    run.block().unwrap();
    RunStore::save(&store, &run).await.unwrap();
    let approval = Approval::new(
        ApprovalId::new("permission-orphan").unwrap(),
        run_id.clone(),
        ApprovalKind::Permission,
        serde_json::json!({
            "schema_version": 1,
            "request_id": "orphan",
            "summary": "Modify files inside the project workspace",
        })
        .to_string(),
    );
    ApprovalStore::save(&store, &approval).await.unwrap();
    // No channel state or live responder simulates the fresh server process.

    let handle = driver::spawn_every(state, Duration::from_millis(20));
    assert_eq!(wait_terminal(&store, &run_id).await, RunStatus::Error);
    let closed = ApprovalStore::get(&store, &approval.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(closed.status, ApprovalStatus::Rejected);
    assert!(closed
        .decision_comment
        .as_deref()
        .unwrap()
        .contains("server restart"));
    assert!(store.list_pending().await.unwrap().is_empty());
    handle.abort();
}

#[tokio::test]
async fn startup_recovery_closes_all_process_claims_before_serving() {
    let (state, store, _agents) = state().await;
    let app = router(state.clone());
    let project = create_project(&app).await;
    let run_id = seed_running_run(&store, &project, "r-startup").await;
    let mut run = RunStore::get(&store, &run_id).await.unwrap().unwrap();
    run.block().unwrap();
    RunStore::save(&store, &run).await.unwrap();
    let permission = Approval::new(
        ApprovalId::new("permission-startup").unwrap(),
        run_id.clone(),
        ApprovalKind::Permission,
        serde_json::json!({
            "schema_version": 1,
            "request_id": "startup",
            "summary": "Modify files inside the project workspace",
        })
        .to_string(),
    );
    ApprovalStore::save(&store, &permission).await.unwrap();

    let mut preview = Preview::new(
        PreviewId::new("preview-startup").unwrap(),
        ProjectId::new(&project).unwrap(),
        4100,
        "work",
    );
    preview.mark_ready(4242).unwrap();
    PreviewStore::save(&store, &preview).await.unwrap();

    let recovered = driver::recover_startup(&state).await.unwrap();
    assert_eq!(recovered.runs, 1);
    assert_eq!(recovered.blocked_permissions, 1);
    assert_eq!(recovered.previews, 1);
    assert_eq!(run_status(&store, &run_id).await, RunStatus::Error);
    assert_eq!(
        ApprovalStore::get(&store, &permission.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Rejected
    );
    assert!(store.active_previews().await.unwrap().is_empty());
    assert_eq!(
        TaskStore::get(&store, &run.task_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Ready
    );
    assert!(store
        .events_since(0)
        .await
        .unwrap()
        .iter()
        .any(|(_, event)| event.kind == EventKind::PreviewError
            && event.payload.contains("restart_preview")));

    assert_eq!(
        driver::recover_startup(&state).await.unwrap(),
        driver::RecoverySummary {
            runs: 0,
            blocked_permissions: 0,
            previews: 0,
        }
    );
}

#[tokio::test]
async fn the_health_loop_marks_a_dead_preview_as_error() {
    let (state, store, _agents) = state().await;
    let app = router(state.clone());
    let project = create_project(&app).await;
    let mut preview = Preview::new(
        PreviewId::new("preview-dead").unwrap(),
        ProjectId::new(&project).unwrap(),
        4100,
        "work",
    );
    preview.mark_ready(4242).unwrap();
    PreviewStore::save(&store, &preview).await.unwrap();

    let handle = driver::spawn_every(state, Duration::from_millis(20));
    for _ in 0..100 {
        if store.active_previews().await.unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(store.active_previews().await.unwrap().is_empty());
    assert!(store
        .events_since(0)
        .await
        .unwrap()
        .iter()
        .any(|(_, event)| event.kind == EventKind::PreviewError
            && event.payload.contains("process_exited")));
    handle.abort();
}

#[tokio::test]
async fn a_finished_run_drives_review_and_journals() {
    let (state, store, agents) = state().await;
    let app = router(state.clone());
    let project = create_project(&app).await;
    let run = seed_running_run(&store, &project, "r-fin").await;

    let handle = driver::spawn_every(state, Duration::from_millis(30));
    // The "agent" completes its turn.
    agents.run_states.lock().unwrap().insert(
        "r-fin".into(),
        RunState::Done(RunReport {
            outcome: RunOutcome::Finished,
            summary: "endpoint implemented".into(),
            activity: vec!["finished: cargo test".into()],
            base_sha: Some("1111111".into()),
            head_sha: Some("2222222".into()),
            commits: vec![CommitEvidence {
                sha: "2222222".into(),
                subject: "feat: implement endpoint".into(),
            }],
            changed_files: vec![ChangedFileEvidence {
                status: "M".into(),
                path: "src/endpoint.rs".into(),
            }],
            diff_stat: "1 file changed, 12 insertions(+)".into(),
        }),
    );

    assert_eq!(wait_terminal(&store, &run).await, RunStatus::Finished);
    let stored_run = RunStore::get(&store, &run).await.unwrap().unwrap();
    assert_eq!(stored_run.summary.as_deref(), Some("endpoint implemented"));
    assert_eq!(stored_run.base_sha.as_deref(), Some("1111111"));
    assert_eq!(stored_run.head_sha.as_deref(), Some("2222222"));
    let artifacts: serde_json::Value =
        serde_json::from_str(stored_run.artifacts.as_deref().unwrap()).unwrap();
    assert_eq!(artifacts["changed_files"][0]["path"], "src/endpoint.rs");
    assert_eq!(artifacts["activity"][0], "finished: cargo test");
    let task = TaskStore::get(&store, &TaskId::new("t-r-fin").unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    // Executor completion alone never creates the human decision.
    assert!(store.list_pending().await.unwrap().is_empty());

    let kinds: Vec<_> = store
        .events_since(0)
        .await
        .unwrap()
        .iter()
        .map(|(_, e)| e.kind)
        .collect();
    assert!(kinds.contains(&EventKind::RunFinished));
    assert!(!kinds.contains(&EventKind::ApprovalRequested));

    // §5.2: the reviewer run is dispatched on the task — after the preview
    // step, so give the tick a moment to get there.
    let mut reviewer = None;
    for _ in 0..100 {
        let runs = RunStore::list_for_task(&store, &task.id).await.unwrap();
        reviewer = runs.into_iter().find(|r| r.role_id.as_str() == "reviewer");
        if reviewer.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let reviewer = reviewer.expect("a reviewer run should have been dispatched");
    assert_eq!(reviewer.status, RunStatus::Running);
    assert!(store.list_pending().await.unwrap().is_empty());

    let prompt = {
        let prompts = agents.run_prompts.lock().unwrap();
        prompts
            .iter()
            .find(|(role, _)| role == "reviewer")
            .map(|(_, prompt)| prompt.clone())
            .expect("the Reviewer prompt was recorded")
    };
    assert!(prompt.contains("Page de connexion"), "{prompt}");
    assert!(prompt.contains("spec s1 v1 (approved)"), "{prompt}");
    assert!(prompt.contains("1111111"), "{prompt}");
    assert!(prompt.contains("2222222"), "{prompt}");
    assert!(prompt.contains("src/endpoint.rs"), "{prompt}");
    assert!(prompt.contains("latoile-review"), "{prompt}");
    let reviewer_result = serde_json::json!({
        "schema_version": 1,
        "verdict": "approve_with_reservations",
        "summary": "Conforme avec une réserve non bloquante.",
        "findings": [{
            "severity": "reservation",
            "text": "Ajouter un état de chargement.",
            "location": "web/src/Login.tsx:42",
            "fix": "Désactiver le bouton pendant la requête."
        }],
        "suggested_follow_ups": ["Ajouter le test de double clic."]
    })
    .to_string();
    agents.run_states.lock().unwrap().insert(
        reviewer.id.as_str().into(),
        RunState::Done(RunReport::terminal(RunOutcome::Finished, reviewer_result)),
    );
    assert_eq!(
        wait_terminal(&store, &reviewer.id).await,
        RunStatus::Finished
    );

    let pending = store.list_pending().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, latoile_core::ApprovalKind::Review);
    assert_eq!(pending[0].run_id, reviewer.id);
    let payload: serde_json::Value = serde_json::from_str(&pending[0].payload).unwrap();
    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["verdict"], "approve_with_reservations");
    // Re-read: the reviewer's RunStarted lands after the snapshot above.
    let later: Vec<_> = store
        .events_since(0)
        .await
        .unwrap()
        .iter()
        .map(|(_, e)| e.kind)
        .collect();
    assert!(later.contains(&EventKind::RunStarted));
    assert!(later.contains(&EventKind::ApprovalRequested));
    handle.abort();
}

#[tokio::test]
async fn a_failed_run_is_journaled_without_a_review() {
    let (state, store, agents) = state().await;
    let app = router(state.clone());
    let project = create_project(&app).await;
    let run = seed_running_run(&store, &project, "r-bad").await;

    let handle = driver::spawn_every(state, Duration::from_millis(30));
    agents
        .run_states
        .lock()
        .unwrap()
        .insert("r-bad".into(), RunState::Failed("process died".into()));

    assert_eq!(wait_terminal(&store, &run).await, RunStatus::Error);
    assert!(store.list_pending().await.unwrap().is_empty());
    let events = store.events_since(0).await.unwrap();
    let failure = events
        .iter()
        .find(|(_, e)| e.kind == EventKind::RunFinished)
        .unwrap();
    assert!(failure.1.payload.contains("process died"));
    handle.abort();
}

#[tokio::test]
async fn a_run_the_channel_never_saw_is_lost_to_the_restart() {
    let (state, store, _agents) = state().await;
    let app = router(state.clone());
    let project = create_project(&app).await;
    let run = seed_running_run(&store, &project, "r-lost").await;
    // No entry in run_states: the channel restarted and forgot everything.

    let handle = driver::spawn_every(state, Duration::from_millis(30));
    assert_eq!(wait_terminal(&store, &run).await, RunStatus::Error);
    let events = store.events_since(0).await.unwrap();
    assert!(events
        .iter()
        .any(|(_, e)| e.payload.contains("server restart")));
    handle.abort();
}

#[tokio::test]
async fn an_active_run_is_left_alone() {
    let (state, store, agents) = state().await;
    let app = router(state.clone());
    let project = create_project(&app).await;
    let run = seed_running_run(&store, &project, "r-run").await;
    agents
        .run_states
        .lock()
        .unwrap()
        .insert("r-run".into(), RunState::Running);

    let handle = driver::spawn_every(state, Duration::from_millis(30));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(run_status(&store, &run).await, RunStatus::Running);
    assert!(store.list_pending().await.unwrap().is_empty());
    handle.abort();
}
