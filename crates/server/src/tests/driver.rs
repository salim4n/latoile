//! Supervision driver tests: scripted channel states, real store, short
//! poll interval. Covers finish → review flow, failure journaling, and the
//! restart-lost case.

use super::*;
use crate::driver;
use latoile_agents::{
    ChangedFileEvidence, CommitEvidence, RunOutcome, RunReport, RunState,
};
use latoile_core::event::EventKind;
use latoile_core::ids::{RunId, SpecVersionId, TaskId};
use latoile_core::ports::{ApprovalStore, RunStore, SpecStore, TaskStore};
use latoile_core::{RoleId, Run, RunStatus, SpecVersion, Task, TaskStatus, TriggeredBy};
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

#[tokio::test]
async fn a_finished_run_drives_review_and_journals() {
    let (state, store, agents) = state().await;
    let app = router(state.clone());
    let project = create_project(&app).await;
    let run = seed_running_run(&store, &project, "r-fin").await;

    let handle = driver::spawn_every(state, Duration::from_millis(30));
    // The "agent" completes its turn.
    agents
        .run_states
        .lock()
        .unwrap()
        .insert(
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
    let pending = store.list_pending().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, latoile_core::ApprovalKind::Review);

    let kinds: Vec<_> = store
        .events_since(0)
        .await
        .unwrap()
        .iter()
        .map(|(_, e)| e.kind)
        .collect();
    assert!(kinds.contains(&EventKind::RunFinished));
    assert!(kinds.contains(&EventKind::ApprovalRequested));

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
    // Re-read: the reviewer's RunStarted lands after the snapshot above.
    let later: Vec<_> = store
        .events_since(0)
        .await
        .unwrap()
        .iter()
        .map(|(_, e)| e.kind)
        .collect();
    assert!(later.contains(&EventKind::RunStarted));
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
