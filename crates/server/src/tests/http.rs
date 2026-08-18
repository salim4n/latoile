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
    let health = body_json(health).await;
    assert_eq!(health["status"], "ok");
    assert_eq!(health["database"], "ok");

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
    assert!(
        projects[0]["last_activity_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );

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
                "name": "X", "slug": "x", "github_repo": "no-slash"
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
    assert!(
        thread
            .as_array()
            .unwrap()
            .iter()
            .all(|message| message["created_at"].as_str().unwrap().ends_with('Z'))
    );
    let events = store.events_since(0).await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|(_, e)| e.kind == EventKind::MessagePosted)
    );
}

#[tokio::test]
async fn an_architecture_brief_starts_a_persistent_socratic_session() {
    let (state, _, agents) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    let started = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Construire un portail de facturation pour cabinets.",
                "intent": "architecture_brief"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);
    let started = body_json(started).await;
    assert_eq!(started["message"]["author"], "user");
    assert_eq!(started["reply"]["author"], "manager");
    assert!(
        started["reply"]["content"]
            .as_str()
            .unwrap()
            .contains("Architecte")
    );

    assert!(agents.manager_messages.lock().unwrap().is_empty());
    assert_eq!(
        agents.architecture_messages.lock().unwrap().as_slice(),
        ["brief:Construire un portail de facturation pour cabinets."]
    );

    let architecture = app
        .clone()
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    assert_eq!(architecture.status(), StatusCode::OK);
    let architecture = body_json(architecture).await;
    assert_eq!(architecture["status"], "awaiting_answer");
    assert_eq!(architecture["phase"], "domain_discovery");
    assert_eq!(architecture["questions"].as_array().unwrap().len(), 1);
    assert_eq!(architecture["questions"][0]["status"], "open");
    assert_eq!(
        architecture["questions"][0]["prompt"],
        "Quel problème doit disparaître pour l'utilisateur ?"
    );

    let answered = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Les relances manuelles et les erreurs de suivi."
            })),
        )))
        .await
        .unwrap();
    assert_eq!(answered.status(), StatusCode::OK);
    let answered = body_json(answered).await;
    assert!(
        answered["reply"]["content"]
            .as_str()
            .unwrap()
            .contains("paquet confiné et vérifié")
    );
    assert!(agents.manager_messages.lock().unwrap().is_empty());
    assert_eq!(
        agents.architecture_messages.lock().unwrap().as_slice(),
        [
            "brief:Construire un portail de facturation pour cabinets.",
            "answer:Les relances manuelles et les erreurs de suivi."
        ]
    );

    let architecture = app
        .clone()
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    let architecture = body_json(architecture).await;
    assert_eq!(architecture["status"], "ready_to_draft");
    assert_eq!(architecture["phase"], "ready_to_draft");
    assert_eq!(architecture["package_status"], "draft_ready");
    assert_eq!(architecture["skill_name"], "app-architect-brainstorm");
    assert_eq!(architecture["skill_digest"].as_str().unwrap().len(), 64);
    assert_eq!(architecture["operating_mode"], "greenfield");
    assert_eq!(
        architecture["package"]["head_sha"],
        "2222222222222222222222222222222222222222"
    );
    assert_eq!(architecture["questions"][0]["status"], "answered");
    assert_eq!(
        architecture["questions"][0]["answer"],
        "Les relances manuelles et les erreurs de suivi."
    );

    let specs = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/spec-versions"),
            None,
        )))
        .await
        .unwrap();
    let specs = body_json(specs).await;
    assert_eq!(specs.as_array().unwrap().len(), 1);
    assert_eq!(specs[0]["status"], "draft");
    assert_eq!(specs[0]["skill_digest"], architecture["skill_digest"]);
    assert_eq!(
        specs[0]["package_digest"],
        architecture["package"]["package_digest"]
    );
    assert_eq!(
        specs[0]["package_commit_sha"],
        architecture["package"]["head_sha"]
    );
}

#[tokio::test]
async fn a_first_turn_ready_signal_is_recentered_into_a_real_question() {
    let (state, _, agents) = state().await;
    {
        let mut replies = agents.architecture_replies.lock().unwrap();
        replies.push_front(
            "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"question\",\"phase\":\"domain_discovery\",\"message\":\"Quelle décision métier doit rester sous contrôle humain ?\"}\n```"
                .into(),
        );
        replies.push_front(
            "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"ready_to_draft\",\"phase\":\"ready_to_draft\",\"message\":\"Le brief semble complet.\"}\n```"
                .into(),
        );
    }
    let app = router(state);
    let project = create_project(&app).await;

    let started = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Construire un produit au brief très détaillé.",
                "intent": "architecture_brief"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);
    assert_eq!(
        agents.architecture_messages.lock().unwrap().as_slice(),
        [
            "brief:Construire un produit au brief très détaillé.",
            "guard:first-question-required"
        ]
    );

    let architecture = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    let architecture = body_json(architecture).await;
    assert_eq!(architecture["status"], "awaiting_answer");
    assert_eq!(architecture["questions"].as_array().unwrap().len(), 1);
    assert_eq!(
        architecture["questions"][0]["prompt"],
        "Quelle décision métier doit rester sous contrôle humain ?"
    );
}

#[tokio::test]
async fn an_architect_that_skips_the_first_challenge_twice_fails_closed() {
    let (state, _, agents) = state().await;
    let ready = "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"ready_to_draft\",\"phase\":\"ready_to_draft\",\"message\":\"Le brief semble complet.\"}\n```";
    {
        let mut replies = agents.architecture_replies.lock().unwrap();
        replies.push_front(ready.into());
        replies.push_front(ready.into());
    }
    let app = router(state);
    let project = create_project(&app).await;

    let refused = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Construire un produit au brief très détaillé.",
                "intent": "architecture_brief"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let architecture = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    let architecture = body_json(architecture).await;
    assert_eq!(architecture["status"], "failed");
    assert!(architecture["failure_reason"]
        .as_str()
        .unwrap()
        .contains("ignored the mandatory first owner challenge twice"));
}

#[tokio::test]
async fn an_answer_contract_phase_mismatch_is_repaired_in_the_same_session() {
    let (state, _, agents) = state().await;
    {
        let mut replies = agents.architecture_replies.lock().unwrap();
        replies.clear();
        replies.extend([
            "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"question\",\"phase\":\"domain_discovery\",\"message\":\"Quel résultat métier doit être mesuré ?\"}\n```".into(),
            "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"ready_to_draft\",\"phase\":\"ux_discovery\",\"message\":\"Les décisions sont suffisantes.\"}\n```".into(),
            "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"ready_to_draft\",\"phase\":\"ready_to_draft\",\"message\":\"Les décisions sont suffisantes.\"}\n```".into(),
        ]);
    }
    let app = router(state);
    let project = create_project(&app).await;

    let started = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Construire un tableau de bord métier.",
                "intent": "architecture_brief"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);

    let answered = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({"content": "Le taux de résolution hebdomadaire."})),
        )))
        .await
        .unwrap();
    assert_eq!(answered.status(), StatusCode::OK);
    assert_eq!(
        agents.architecture_messages.lock().unwrap().as_slice(),
        [
            "brief:Construire un tableau de bord métier.",
            "answer:Le taux de résolution hebdomadaire.",
            "guard:contract-repair:domain_discovery"
        ]
    );

    let architecture = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    let architecture = body_json(architecture).await;
    assert_eq!(architecture["status"], "ready_to_draft");
    assert_eq!(architecture["phase"], "ready_to_draft");
    assert_eq!(architecture["package_status"], "draft_ready");
}

#[tokio::test]
async fn a_second_answer_contract_mismatch_fails_closed() {
    let (state, _, agents) = state().await;
    let mismatch = "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"ready_to_draft\",\"phase\":\"ux_discovery\",\"message\":\"Les décisions sont suffisantes.\"}\n```";
    {
        let mut replies = agents.architecture_replies.lock().unwrap();
        replies.clear();
        replies.extend([
            "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"question\",\"phase\":\"domain_discovery\",\"message\":\"Quel résultat métier doit être mesuré ?\"}\n```".into(),
            mismatch.into(),
            mismatch.into(),
        ]);
    }
    let app = router(state);
    let project = create_project(&app).await;
    app.clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Construire un tableau de bord métier.",
                "intent": "architecture_brief"
            })),
        )))
        .await
        .unwrap();

    let refused = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({"content": "Le taux de résolution hebdomadaire."})),
        )))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let architecture = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    let architecture = body_json(architecture).await;
    assert_eq!(architecture["status"], "failed");
    assert!(architecture["failure_reason"]
        .as_str()
        .unwrap()
        .contains("contract repair failed: question/ready phase mismatch"));
    assert_eq!(
        agents
            .architecture_messages
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message.starts_with("guard:contract-repair:"))
            .count(),
        1
    );
}

#[tokio::test]
async fn architecture_discovery_can_be_cancelled_and_stays_observable() {
    let (state, _, agents) = state().await;
    let app = router(state);
    let project = create_project(&app).await;
    app.clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Construire un portail client.",
                "intent": "architecture_brief"
            })),
        )))
        .await
        .unwrap();

    let cancelled = app
        .clone()
        .oneshot(authed(request(
            "DELETE",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
    let cancelled = body_json(cancelled).await;
    assert_eq!(cancelled["status"], "cancelled");
    assert!(
        agents
            .architecture_messages
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .starts_with("cancel:")
    );

    let retry = app
        .oneshot(authed(request(
            "DELETE",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn malformed_architect_output_fails_closed_without_waking_the_manager() {
    let (state, _, agents) = state().await;
    agents
        .architecture_replies
        .lock()
        .unwrap()
        .push_front("Je vais directement coder le produit.".into());
    let app = router(state);
    let project = create_project(&app).await;

    let refused = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Construire une marketplace.",
                "intent": "architecture_brief"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(agents.manager_messages.lock().unwrap().is_empty());

    let architecture = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    let architecture = body_json(architecture).await;
    assert_eq!(architecture["status"], "failed");
    assert!(
        architecture["failure_reason"]
            .as_str()
            .unwrap()
            .contains("latoile-architecture")
    );
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
    assert!(
        body_json(response).await["message"]
            .as_str()
            .unwrap()
            .contains("spec")
    );
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
    crate::tests::approve_test_spec(&store, &mut spec).await;
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
    crate::tests::approve_test_spec(&store, &mut spec).await;
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
