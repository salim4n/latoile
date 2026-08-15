//! Channel tests: scripted connections, no processes.

use super::*;
use crate::config::{AgentCommand, AgentTimeouts};
use crate::transport::TurnResult;
use crate::updates::AgentUpdate;
use latoile_core::ids::{RoleId, TaskId};
use latoile_core::TriggeredBy;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// A scripted connection. `pend` makes prompt wait forever — the
/// cancellation and timeout tests' wedge.
struct FakeConn {
    log: Arc<StdMutex<Vec<String>>>,
    queued: VecDeque<Result<TurnResult, AgentError>>,
    pend: bool,
    dropped: Arc<AtomicBool>,
}

#[allow(clippy::manual_async_fn)] // matching the trait's Send-bound shape
impl Connection for FakeConn {
    fn new_session<'a>(
        &'a mut self,
        _cwd: &'a Path,
    ) -> impl std::future::Future<Output = Result<(), AgentError>> + Send + 'a {
        async move { Ok(()) }
    }
    fn prompt<'a>(
        &'a mut self,
        text: &'a str,
    ) -> impl std::future::Future<Output = Result<TurnResult, AgentError>> + Send + 'a {
        async move {
            self.log.lock().unwrap().push(text.to_string());
            if self.pend {
                std::future::pending::<()>().await;
            }
            self.queued.pop_front().unwrap_or_else(|| {
                Ok(TurnResult {
                    outcome: RunOutcome::Finished,
                    text: "réponse".into(),
                    updates: vec![AgentUpdate::TextChunk("réponse".into())],
                })
            })
        }
    }
    fn cancel(
        &mut self,
    ) -> impl std::future::Future<Output = Result<(), AgentError>> + Send + '_ {
        async move { Ok(()) }
    }
}

impl Drop for FakeConn {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct FakeConnector {
    conns: StdMutex<VecDeque<FakeConn>>,
    spawned: AtomicUsize,
}

impl FakeConnector {
    fn push(&self, conn: FakeConn) {
        self.conns.lock().unwrap().push_back(conn);
    }
}

#[allow(clippy::manual_async_fn)] // matching the trait's Send-bound shape
impl Connector for FakeConnector {
    type Conn = FakeConn;
    fn connect<'a>(
        &'a self,
        _command: &'a AgentCommand,
        _workspace: &'a Path,
    ) -> impl std::future::Future<Output = Result<FakeConn, AgentError>> + Send + 'a {
        async move {
            self.spawned.fetch_add(1, Ordering::SeqCst);
            Ok(self.conns.lock().unwrap().pop_front().unwrap_or(FakeConn {
                log: Arc::new(StdMutex::new(vec![])),
                queued: VecDeque::new(),
                pend: false,
                dropped: Arc::new(AtomicBool::new(false)),
            }))
        }
    }
}

fn fixture() -> (tempfile::TempDir, ChannelConfig) {
    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("project-manager");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "SKILL MANAGER").unwrap();
    let config = ChannelConfig {
        skills_dir: dir.path().to_path_buf(),
        ..ChannelConfig::default()
    };
    (dir, config)
}

fn channel(
    config: ChannelConfig,
    connector: FakeConnector,
    root: &Path,
) -> AcpChannel<FakeConnector, RootDirs> {
    AcpChannel::new(config, connector, RootDirs(root.to_path_buf()))
}

fn project() -> ProjectId {
    ProjectId::new("p1").unwrap()
}

fn run(id: &str) -> Run {
    Run::new(
        RunId::new(id).unwrap(),
        TaskId::new("t1").unwrap(),
        RoleId::new("backend").unwrap(),
        TriggeredBy::Manager,
    )
}

async fn wait_for<C: Connector, D: ProjectDirs>(
    ch: &AcpChannel<C, D>,
    run: &RunId,
    want: impl Fn(&RunState) -> bool,
) -> RunState {
    for _ in 0..100 {
        if let Some(state) = ch.run_state(run).await {
            if want(&state) {
                return state;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run never reached the expected state");
}

#[tokio::test]
async fn the_manager_answers_and_the_preamble_heads_the_first_prompt_only() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    let log = Arc::new(StdMutex::new(Vec::new()));
    connector.push(FakeConn {
        log: log.clone(),
        queued: VecDeque::new(),
        pend: false,
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let ch = channel(config, connector, dir.path());

    let reply = ch.tell_manager(&project(), "construis la page").await.unwrap();
    assert_eq!(reply.content, "réponse");
    assert_eq!(reply.actions, None);

    ch.tell_manager(&project(), "et le formulaire ?").await.unwrap();
    let prompts = log.lock().unwrap();
    assert!(prompts[0].starts_with("SKILL MANAGER\n\n---\n\n"));
    assert_eq!(prompts[1], "et le formulaire ?", "no preamble twice");
}

#[tokio::test]
async fn a_wedged_manager_is_evicted_and_the_next_message_respawns() {
    let (dir, mut config) = fixture();
    config.timeouts = AgentTimeouts {
        prompt: Duration::from_millis(50),
        ..AgentTimeouts::default()
    };
    let connector = FakeConnector::default();
    let wedged = Arc::new(AtomicBool::new(false));
    connector.push(FakeConn {
        log: Arc::new(StdMutex::new(vec![])),
        queued: VecDeque::new(),
        pend: true,
        dropped: wedged.clone(),
    });
    let ch = channel(config, connector, dir.path());

    let err = ch.tell_manager(&project(), "allo").await;
    assert!(err.is_err(), "a wedged manager must not hang the caller");
    assert!(wedged.load(Ordering::SeqCst), "the wedged process is killed");

    // Next message: a fresh session answers.
    let reply = ch.tell_manager(&project(), "allo").await.unwrap();
    assert_eq!(reply.content, "réponse");
}

#[tokio::test]
async fn a_run_completes_in_the_background() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    let ch = channel(config, connector, dir.path());
    let r = run("r1");

    let handle = ch.start_run(&r, "implémente le endpoint").await.unwrap();
    assert_eq!(handle, "acp:r1");

    let state = wait_for(&ch, &r.id, |s| !matches!(s, RunState::Running)).await;
    assert_eq!(state, RunState::Done(RunOutcome::Finished));
}

#[tokio::test]
async fn a_run_that_dies_mid_stream_records_a_failure_not_a_hang() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    connector.push(FakeConn {
        log: Arc::new(StdMutex::new(vec![])),
        queued: VecDeque::from([Err(AgentError::AgentGone)]),
        pend: false,
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let ch = channel(config, connector, dir.path());
    let r = run("r2");

    ch.start_run(&r, "travaille").await.unwrap();
    let state = wait_for(&ch, &r.id, |s| !matches!(s, RunState::Running)).await;
    assert!(matches!(state, RunState::Failed(_)), "{state:?}");
}

#[tokio::test]
async fn cancelling_a_run_kills_its_process() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    let dropped = Arc::new(AtomicBool::new(false));
    connector.push(FakeConn {
        log: Arc::new(StdMutex::new(vec![])),
        queued: VecDeque::new(),
        pend: true,
        dropped: dropped.clone(),
    });
    let ch = channel(config, connector, dir.path());
    let r = run("r3");

    ch.start_run(&r, "travaille").await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await; // let it start
    ch.cancel_run(&r.id).await.unwrap();

    assert_eq!(
        ch.run_state(&r.id).await,
        None,
        "a cancelled run leaves the registry"
    );
    for _ in 0..100 {
        if dropped.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the cancelled run's process was not killed");
}

#[tokio::test]
async fn a_run_past_its_time_budget_is_killed() {
    let (dir, mut config) = fixture();
    config.timeouts = AgentTimeouts {
        prompt: Duration::from_millis(50),
        ..AgentTimeouts::default()
    };
    let connector = FakeConnector::default();
    connector.push(FakeConn {
        log: Arc::new(StdMutex::new(vec![])),
        queued: VecDeque::new(),
        pend: true,
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let ch = channel(config, connector, dir.path());
    let r = run("r4");

    ch.start_run(&r, "travaille").await.unwrap();
    let state = wait_for(&ch, &r.id, |s| !matches!(s, RunState::Running)).await;
    assert_eq!(state, RunState::Failed("timed out".into()));
}

#[tokio::test]
async fn cancelling_an_unknown_run_is_fine() {
    let (dir, config) = fixture();
    let ch = channel(config, FakeConnector::default(), dir.path());
    ch.cancel_run(&RunId::new("ghost").unwrap()).await.unwrap();
}
