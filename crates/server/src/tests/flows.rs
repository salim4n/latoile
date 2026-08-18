//! Flow tests: approvals, spec approval, SSE resume, preview proxy, roles.

use super::*;
use axum::http::StatusCode;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{ApprovalId, RunId, SpecVersionId, TaskId};
use latoile_core::ports::{ApprovalStore, EventLog, RunStore, SpecStore, TaskStore};
use latoile_core::{Approval, ApprovalKind, Preview, PreviewId, SpecVersion, Task};
use latoile_core::{RoleId, TriggeredBy};
use tower::ServiceExt;

/// Seed the fixture chain project → approved spec → task → run, with the
/// task driven to `review`.
async fn seed_review_pending(store: &Store, project: &str) -> Approval {
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
        TaskId::new("t1").unwrap(),
        ProjectId::new(project).unwrap(),
        RoleId::new("frontend").unwrap(),
        "Page de connexion",
        "Formulaire",
        0,
    )
    .unwrap();
    task.bind_spec(spec.id.clone());
    task.start().unwrap();
    task.submit_for_review().unwrap();
    TaskStore::save(store, &task).await.unwrap();

    let mut run = Run::new(
        RunId::new("r1").unwrap(),
        task.id.clone(),
        RoleId::new("frontend").unwrap(),
        TriggeredBy::Manager,
    );
    run.begin().unwrap();
    RunStore::save(store, &run).await.unwrap();

    let approval = Approval::new(
        ApprovalId::new("a1").unwrap(),
        run.id,
        ApprovalKind::Review,
        "{}".into(),
    );
    ApprovalStore::save(store, &approval).await.unwrap();
    approval
}

#[tokio::test]
async fn a_granted_review_closes_the_task() {
    let (state, store, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;
    let approval = seed_review_pending(&store, &project).await;

    let pending = app
        .clone()
        .oneshot(authed(request("GET", "/api/approvals", None)))
        .await
        .unwrap();
    let pending = body_json(pending).await;
    assert_eq!(pending.as_array().unwrap().len(), 1);
    assert_eq!(pending[0]["project_id"], project);
    assert_eq!(pending[0]["project_name"], "Mon App");
    assert_eq!(pending[0]["task_title"], "Page de connexion");
    assert_eq!(pending[0]["role_id"], "frontend");
    assert!(pending[0]["created_at"].as_str().unwrap().ends_with('Z'));

    let decided = app
        .clone()
        .oneshot(authed(request(
            "POST",
            "/api/approvals/a1",
            Some(serde_json::json!({"granted": true})),
        )))
        .await
        .unwrap();
    assert_eq!(body_json(decided).await["status"], "granted");
    assert_eq!(approval.kind, ApprovalKind::Review);

    let task = TaskStore::get(&store, &TaskId::new("t1").unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.status, latoile_core::TaskStatus::Done);
}

#[tokio::test]
async fn a_rejected_review_leaves_the_task_in_review() {
    let (state, store, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;
    seed_review_pending(&store, &project).await;

    let decided = app
        .clone()
        .oneshot(authed(request(
            "POST",
            "/api/approvals/a1",
            Some(serde_json::json!({"granted": false})),
        )))
        .await
        .unwrap();
    assert_eq!(body_json(decided).await["status"], "rejected");

    let task = TaskStore::get(&store, &TaskId::new("t1").unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.status, latoile_core::TaskStatus::Review);

    let pending = app
        .oneshot(authed(request("GET", "/api/approvals", None)))
        .await
        .unwrap();
    assert!(body_json(pending).await.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn approving_a_spec_marks_the_project_specced() {
    let (state, store, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    let draft = SpecVersion::new(
        SpecVersionId::new("s1").unwrap(),
        ProjectId::new(&project).unwrap(),
        1,
        "design/",
        None,
    )
    .unwrap();
    SpecStore::save(&store, &draft).await.unwrap();

    let list = app
        .clone()
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/spec-versions"),
            None,
        )))
        .await
        .unwrap();
    assert_eq!(body_json(list).await[0]["status"], "draft");

    let approved = app
        .clone()
        .oneshot(authed(request(
            "POST",
            "/api/spec-versions/s1/approve",
            None,
        )))
        .await
        .unwrap();
    assert_eq!(approved.status(), StatusCode::OK);
    assert_eq!(body_json(approved).await["status"], "approved");

    let detail = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}"),
            None,
        )))
        .await
        .unwrap();
    assert_eq!(body_json(detail).await["status"], "specced");
}

#[tokio::test]
async fn the_event_stream_replays_what_was_missed() {
    let (state, store, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;
    let project_id = ProjectId::new(&project).unwrap();

    for i in 0..2 {
        EventLog::append(
            &store,
            &NewEvent {
                project_id: project_id.clone(),
                kind: EventKind::TaskReady,
                payload: format!("{{\"n\":{i}}}"),
            },
        )
        .await
        .unwrap();
    }

    let response = app
        .oneshot(authed(request("GET", "/api/events?after=0", None)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Read frames until both events arrived, then hang up.
    let mut body = response.into_body();
    let mut text = String::new();
    for _ in 0..20 {
        let frame = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            http_body_util::BodyExt::frame(&mut body),
        )
        .await
        .expect("the SSE stream stalled")
        .expect("the stream ended early")
        .unwrap();
        if let Some(data) = frame.data_ref() {
            text.push_str(&String::from_utf8_lossy(data));
        }
        if text.matches("event: task_ready").count() == 2 {
            break;
        }
    }
    assert_eq!(text.matches("event: task_ready").count(), 2, "{text}");
    assert!(text.contains("\"n\":1"));
}

/// The proxy forwards to the preview's loopback port and streams the body
/// back untouched.
#[tokio::test]
async fn the_preview_proxy_forwards_to_the_dev_server() {
    let (state, store, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    // A throwaway dev server on 127.0.0.1.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await.unwrap();
        let body = "proxied-ok";
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });

    let mut preview = Preview::new(
        PreviewId::new("pr1").unwrap(),
        ProjectId::new(&project).unwrap(),
        port,
        "work",
    );
    preview.mark_ready(4242).unwrap();
    latoile_core::ports::PreviewStore::save(&store, &preview)
        .await
        .unwrap();

    let response = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/preview/index.html"),
            None,
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&bytes[..], b"proxied-ok");
}

#[tokio::test]
async fn the_proxy_404s_when_nothing_is_running() {
    let (state, _, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    let response = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/preview/"),
            None,
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(response).await["code"], "not_found");
}

#[tokio::test]
async fn the_roles_route_lists_the_seeded_team() {
    let (state, _, _) = state().await;
    let app = router(state);

    let response = app
        .oneshot(authed(request("GET", "/api/roles", None)))
        .await
        .unwrap();
    let roles = body_json(response).await;
    assert_eq!(roles.as_array().unwrap().len(), 5);
    assert!(roles
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["id"] == "manager"));
}

/// The documented D9 exception: `?token=` works for preview paths only.
#[tokio::test]
async fn the_query_token_only_opens_preview_paths() {
    let (state, _, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    // Preview path with the query token: authenticated (404 — no preview
    // running — but NOT 401).
    let preview = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/projects/{project}/preview/?token={TOKEN}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::NOT_FOUND);

    // The data API refuses the query token: headers only.
    let data = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/projects?token={TOKEN}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(data.status(), StatusCode::UNAUTHORIZED);

    // And a wrong query token on a preview path is still a 401.
    let wrong = app
        .oneshot(request(
            "GET",
            &format!("/api/projects/{project}/preview/?token=nope"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
}
