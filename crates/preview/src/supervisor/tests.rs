//! Supervisor tests. Real processes, but throwaway ones: a python3 one-liner
//! HTTP server bound to 127.0.0.1, driven through the injected dev command —
//! no project, no repo, no network beyond loopback.

use super::*;
use latoile_core::ids::{PreviewId, ProjectId};
use std::net::{Ipv4Addr, SocketAddr};
use tokio::net::{TcpListener, TcpStream};

/// A dev server that serves on `$PORT`, the way a real project would.
const SERVER_CMD: &str = "python3 -c \"import os,http.server;\
http.server.ThreadingHTTPServer(('127.0.0.1',int(os.environ['PORT'])),\
http.server.SimpleHTTPRequestHandler).serve_forever()\"";
const WORKING_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Each test gets its own 100-port window: two supervisors racing for the
/// same base would both find it bindable before either child has bound, and
/// one child would lose. Distinct windows keep tests parallel-safe.
fn config() -> SupervisorConfig {
    config_at(24100)
}

fn config_at(base_port: u16) -> SupervisorConfig {
    SupervisorConfig {
        base_port,
        readiness: Duration::from_secs(15),
        ..SupervisorConfig::default()
    }
}

fn preview(id: &str, port: u16) -> Preview {
    Preview::new(
        PreviewId::new(id).unwrap(),
        ProjectId::new("p1").unwrap(),
        port,
        "work",
    )
}

async fn listening(port: u16) -> bool {
    TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .is_ok()
}

#[tokio::test]
async fn start_becomes_ready_on_a_serving_port() {
    let sup = Supervisor::new(config());
    let p = preview("pr1", 0);

    let (pid, port) = sup.ensure(&p, SERVER_CMD, WORKING_DIR).await.unwrap();
    assert!(pid > 0);
    assert!(port >= 24100);
    assert!(listening(port).await, "the dev server answers on {port}");
    assert!(sup.is_alive(&p.id).await);

    sup.stop(&p).await.unwrap();
}

#[tokio::test]
async fn stop_frees_the_port_and_the_slot() {
    let sup = Supervisor::new(config_at(24200));
    let p = preview("pr1", 0);

    let (_, port) = sup.ensure(&p, SERVER_CMD, WORKING_DIR).await.unwrap();
    sup.stop(&p).await.unwrap();

    for _ in 0..50 {
        if !listening(port).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        !listening(port).await,
        "the port is still serving after stop"
    );
    assert!(!sup.is_alive(&p.id).await);
}

#[tokio::test]
async fn a_second_ensure_recycles_the_process() {
    let sup = Supervisor::new(config_at(24300));
    let p = preview("pr1", 0);

    let (first_pid, _) = sup.ensure(&p, SERVER_CMD, WORKING_DIR).await.unwrap();
    let (second_pid, port) = sup.ensure(&p, SERVER_CMD, WORKING_DIR).await.unwrap();

    assert_ne!(first_pid, second_pid, "a fresh process, not the old one");
    assert!(listening(port).await);
    sup.stop(&p).await.unwrap();
}

#[tokio::test]
async fn a_refresh_keeps_the_port_so_the_url_survives() {
    let sup = Supervisor::new(config_at(24400));
    let mut p = preview("pr1", 0);

    let (_, port) = sup.ensure(&p, SERVER_CMD, WORKING_DIR).await.unwrap();
    // The use case's refresh path: the Preview entity carries its port.
    p.port = port;
    let (_, again) = sup.ensure(&p, SERVER_CMD, WORKING_DIR).await.unwrap();
    assert_eq!(again, port, "the UI's iframe URL must not move on refresh");
    sup.stop(&p).await.unwrap();
}

#[tokio::test]
async fn a_port_taken_by_something_else_is_skipped() {
    let squat = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 24500)))
        .await
        .unwrap();
    let sup = Supervisor::new(config_at(24500));
    let p = preview("pr1", 0);

    let (_, port) = sup.ensure(&p, SERVER_CMD, WORKING_DIR).await.unwrap();
    assert_ne!(port, 24500);
    assert!(listening(port).await);

    sup.stop(&p).await.unwrap();
    drop(squat);
}

#[tokio::test]
async fn a_server_that_never_listens_is_killed_at_the_budget() {
    let sup = Supervisor::new(SupervisorConfig {
        readiness: Duration::from_millis(500),
        ..config_at(24600)
    });
    let p = preview("pr1", 0);

    let err = sup.ensure(&p, "sleep 30", WORKING_DIR).await.unwrap_err();
    assert!(err.to_string().contains("not ready"), "{err}");

    // The port was handed back. A fresh supervisor with a normal budget:
    // the wedged one's 500 ms is for the failure path only — a cold python
    // under parallel test load needs the real one.
    let sup2 = Supervisor::new(config_at(24600));
    let (_, port) = sup2.ensure(&p, SERVER_CMD, WORKING_DIR).await.unwrap();
    assert_eq!(port, 24600, "the timed-out server's port was not freed");
    assert!(listening(port).await);
    sup2.stop(&p).await.unwrap();
}

#[tokio::test]
async fn a_command_that_exits_immediately_reports_its_last_words() {
    let sup = Supervisor::new(config_at(24600));
    let p = preview("pr1", 0);

    let err = sup
        .ensure(&p, "echo dying-breath >&2; exit 1", WORKING_DIR)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("dying-breath"), "{err}");
}

#[tokio::test]
async fn logs_are_captured_and_exposed() {
    let sup = Supervisor::new(config_at(24700));
    let p = preview("pr1", 0);

    sup.ensure(
        &p,
        "echo first-words; echo cwd:$PWD; exec python3 -c \"import os,http.server;\
http.server.ThreadingHTTPServer(('127.0.0.1',int(os.environ['PORT'])),\
http.server.SimpleHTTPRequestHandler).serve_forever()\"",
        WORKING_DIR,
    )
    .await
    .unwrap();

    let mut found = false;
    for _ in 0..50 {
        if sup
            .logs(&p.id)
            .await
            .iter()
            .any(|l| l.contains("first-words"))
        {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(found, "the dev server's output never reached the ring");
    assert!(sup
        .logs(&p.id)
        .await
        .iter()
        .any(|line| line == &format!("cwd:{WORKING_DIR}")));
    sup.stop(&p).await.unwrap();
}

#[tokio::test]
async fn stopping_an_unknown_preview_is_fine() {
    let sup = Supervisor::new(config_at(24800));
    sup.stop(&preview("ghost", 0)).await.unwrap();
}

#[tokio::test]
async fn two_projects_run_side_by_side_on_distinct_ports() {
    let sup = Supervisor::new(config_at(24900));
    let a = preview("pr-a", 0);
    let b = Preview::new(
        PreviewId::new("pr-b").unwrap(),
        ProjectId::new("p2").unwrap(),
        0,
        "work",
    );

    let (_, port_a) = sup.ensure(&a, SERVER_CMD, WORKING_DIR).await.unwrap();
    let (_, port_b) = sup.ensure(&b, SERVER_CMD, WORKING_DIR).await.unwrap();
    assert_ne!(port_a, port_b);
    assert!(listening(port_a).await && listening(port_b).await);

    // Stopping one leaves the other alone.
    sup.stop(&a).await.unwrap();
    assert!(listening(port_b).await);

    sup.stop(&b).await.unwrap();
}
