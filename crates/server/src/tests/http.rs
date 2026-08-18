//! HTTP contract tests through the assembled router.

use super::*;
use axum::http::StatusCode;
use latoile_core::event::EventKind;
use latoile_core::ports::SpecStore;
use tower::ServiceExt;

#[tokio::test]
async fn health_is_open_and_everything_else_needs_the_token() {
    let (state, _, _) = state().await;
    let app = router(state);

    let health = app
        .clone()
        .oneshot(request("GET", "/api/health", None))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);

    let refused = app
        .oneshot(request("GET", "/api/projects", None))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let body = body_json(refused).await;
    assert_eq!(body["code"], "unauthorized");
}

#[tokio::test]
async fn repository_picker_exposes_visibility_without_exposing_credentials() {
    let (state, _, _) = state().await;
    let app = router(state);

    let response = app
        .oneshot(authed(request("GET", "/api/github/repos", None)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let repositories = body_json(response).await;
    assert_eq!(repositories[0]["full_name"], "salim4n/mon-app");
    assert_eq!(repositories[0]["private"], true);
    assert!(repositories[0].get("token").is_none());
}

#[tokio::test]
async fn a_project_is_created_and_listed() {
    let (state, _, _) = state().await;
    let app = router(state);

    let id = create_project(&app).await;

    let list = app
        .clone()
        .oneshot(authed(request("GET", "/api/projects", None)))
        .await
        .unwrap();
    let projects = body_json(list).await;
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["slug"], "mon-app");
    assert!(projects[0]["last_activity_at"]
        .as_str()
        .unwrap()
        .ends_with('Z'));

    let detail = app
        .oneshot(authed(request("GET", &format!("/api/projects/{id}"), None)))
        .await
        .unwrap();
    assert_eq!(body_json(detail).await["status"], "draft");
}

#[tokio::test]
async fn a_bad_project_shape_gets_the_contract_error() {
    let (state, _, _) = state().await;
    let app = router(state);

    let response = app
        .oneshot(authed(request(
            "POST",
            "/api/projects",
            Some(serde_json::json!({
                "name": "X", "slug": "x", "github_repo": "no-slash",
                "work_branch": "work", "local_path": "/tmp/x", "dev_command": "dev"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(response).await;
    assert_eq!(body["code"], "domain_refused");
}

#[tokio::test]
async fn a_message_is_stored_and_the_manager_answers() {
    let (state, store, agents) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    let sent = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({"content": "construis la page de connexion"})),
        )))
        .await
        .unwrap();
    assert_eq!(sent.status(), StatusCode::OK);
    let body = body_json(sent).await;
    assert_eq!(body["message"]["author"], "user");
    assert_eq!(body["reply"]["author"], "manager");
    assert_eq!(body["reply"]["content"], "Bien reçu, je m'en occupe.");

    // The manager saw the message, and both sides hit the thread + journal.
    assert_eq!(agents.manager_messages.lock().unwrap().len(), 1);
    let thread = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/messages"),
            None,
        )))
        .await
        .unwrap();
    let thread = body_json(thread).await;
    assert_eq!(thread.as_array().unwrap().len(), 2);
    assert!(thread
        .as_array()
        .unwrap()
        .iter()
        .all(|message| message["created_at"].as_str().unwrap().ends_with('Z')));
    let events = store.events_since(0).await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(events
        .iter()
        .all(|(_, e)| e.kind == EventKind::MessagePosted));
}

#[tokio::test]
async fn dispatch_without_a_spec_is_refused_with_a_domain_error() {
    let (state, _, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    let response = app
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/tasks"),
            Some(serde_json::json!({"role_id": "frontend", "title": "Page de connexion"})),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body_json(response).await["message"]
        .as_str()
        .unwrap()
        .contains("spec"));
}

/// Seed an approved spec, then dispatch: the task starts and its run is on
/// the stub channel.
#[tokio::test]
async fn dispatch_with_an_approved_spec_starts_a_run() {
    let (state, store, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    let mut spec = latoile_core::SpecVersion::new(
        latoile_core::ids::SpecVersionId::new("s1").unwrap(),
        ProjectId::new(&project).unwrap(),
        1,
        "design/",
        None,
    )
    .unwrap();
    spec.approve().unwrap();
    SpecStore::save(&store, &spec).await.unwrap();

    let response = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/tasks"),
            Some(serde_json::json!({
                "role_id": "frontend",
                "title": "Page de connexion",
                "description": "Formulaire email + mot de passe"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let task = body_json(response).await;
    assert_eq!(task["status"], "in_progress");

    let tasks = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/tasks"),
            None,
        )))
        .await
        .unwrap();
    let tasks = body_json(tasks).await;
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    assert!(tasks[0]["latest_run_id"].is_string());

    let events = store.events_since(0).await.unwrap();
    let kinds: Vec<_> = events.iter().map(|(_, e)| e.kind).collect();
    assert_eq!(kinds, [EventKind::TaskReady, EventKind::RunStarted]);
}

/// The orchestration loop end to end: a Manager reply carrying an actions
/// block dispatches a task and starts its run through the HTTP route.
#[tokio::test]
async fn a_manager_reply_with_actions_drives_the_board() {
    let (state, store, agents) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    let mut spec = latoile_core::SpecVersion::new(
        latoile_core::ids::SpecVersionId::new("s1").unwrap(),
        ProjectId::new(&project).unwrap(),
        1,
        "design/",
        None,
    )
    .unwrap();
    spec.approve().unwrap();
    SpecStore::save(&store, &spec).await.unwrap();

    *agents.manager_reply.lock().unwrap() = "Je lance le Frontend.\n\n```latoile-actions\n[{\"type\": \"dispatch_task\", \"title\": \"Page de connexion\", \"role_id\": \"frontend\", \"prompt\": \"Build it per design/\"}]\n```".into();

    let sent = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({"content": "construis la page de connexion"})),
        )))
        .await
        .unwrap();
    let body = body_json(sent).await;
    // The prose only — the block is stripped; the cards carry the action.
    assert_eq!(body["reply"]["content"], "Je lance le Frontend.");
    let cards: serde_json::Value =
        serde_json::from_str(body["reply"]["actions"].as_str().unwrap()).unwrap();
    assert_eq!(cards[0]["title"], "Run started — Page de connexion");

    let tasks = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/tasks"),
            None,
        )))
        .await
        .unwrap();
    let tasks = body_json(tasks).await;
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    assert_eq!(tasks[0]["status"], "in_progress");
}
