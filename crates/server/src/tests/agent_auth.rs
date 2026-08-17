//! Agent-auth route tests: the full click-to-login flow through HTTP with
//! the scripted `sh` login command from the test fixture.

use super::*;
use axum::http::StatusCode;
use tower::ServiceExt;

/// Poll the status route until the condition holds.
async fn poll_status(app: &axum::Router, id: &str, want: &str) -> serde_json::Value {
    for _ in 0..100 {
        let response = app
            .clone()
            .oneshot(authed(request(
                "GET",
                &format!("/api/agent-auth/{id}"),
                None,
            )))
            .await
            .unwrap();
        let body = body_json(response).await;
        if body["status"] == want {
            return body;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("session never reached {want}");
}

#[tokio::test]
async fn the_full_login_flow_over_http() {
    let (state, _, _) = state().await;
    let app = router(state);

    // Start: a session id, Starting or already waiting, no URL guarantee yet.
    let started = app
        .clone()
        .oneshot(authed(request("POST", "/api/agent-auth/start", None)))
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);
    let started = body_json(started).await;
    let id = started["session_id"].as_str().unwrap().to_string();

    // The URL appears, ANSI-free.
    let waiting = poll_status(&app, &id, "waiting_for_input").await;
    assert_eq!(
        waiting["url"].as_str().unwrap(),
        "https://claude.com/oauth/authorize?test=1"
    );

    // A wrong code: accepted for writing, then the session fails.
    let sent = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/agent-auth/{id}/code"),
            Some(serde_json::json!({"code": "bad"})),
        )))
        .await
        .unwrap();
    assert_eq!(body_json(sent).await["status"], "validating");
    poll_status(&app, &id, "failed").await;

    // A fresh session with the right code authenticates.
    let again = app
        .clone()
        .oneshot(authed(request("POST", "/api/agent-auth/start", None)))
        .await
        .unwrap();
    let id = body_json(again).await["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    poll_status(&app, &id, "waiting_for_input").await;
    app.clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/agent-auth/{id}/code"),
            Some(serde_json::json!({"code": "good"})),
        )))
        .await
        .unwrap();
    poll_status(&app, &id, "authenticated").await;
}

#[tokio::test]
async fn unknown_sessions_and_codes_at_the_wrong_time_get_contract_errors() {
    let (state, _, _) = state().await;
    let app = router(state);

    let missing = app
        .clone()
        .oneshot(authed(request("GET", "/api/agent-auth/nope", None)))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(missing).await["code"], "not_found");

    // Code before the URL: 409 not_waiting.
    let started = app
        .clone()
        .oneshot(authed(request("POST", "/api/agent-auth/start", None)))
        .await
        .unwrap();
    let id = body_json(started).await["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    // Race-safe: if the URL already landed, skip — the 409 path is covered
    // by the agents unit tests; here we only accept the two honest outcomes.
    let sent = app
        .oneshot(authed(request(
            "POST",
            &format!("/api/agent-auth/{id}/code"),
            Some(serde_json::json!({"code": "x"})),
        )))
        .await
        .unwrap();
    assert!(
        sent.status() == StatusCode::CONFLICT || sent.status() == StatusCode::OK,
        "{}",
        sent.status()
    );
    if sent.status() == StatusCode::CONFLICT {
        assert_eq!(body_json(sent).await["code"], "not_waiting");
    }
}

#[tokio::test]
async fn agent_auth_routes_need_the_token() {
    let (state, _, _) = state().await;
    let app = router(state);
    let response = app
        .oneshot(request("POST", "/api/agent-auth/start", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_codex_session_carries_url_code_and_no_input() {
    let (state, _, _) = state().await;
    let app = router(state);

    let started = app
        .clone()
        .oneshot(authed(request(
            "POST",
            "/api/agent-auth/start",
            Some(serde_json::json!({"provider": "codex"})),
        )))
        .await
        .unwrap();
    let id = body_json(started).await["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let waiting = poll_status(&app, &id, "waiting_for_input").await;
    assert_eq!(waiting["provider"], "codex");
    assert_eq!(waiting["input_required"], false);
    assert_eq!(
        waiting["url"].as_str().unwrap(),
        "https://auth.openai.com/codex/device"
    );
    assert_eq!(waiting["user_code"].as_str().unwrap(), "TEST-CODE1");

    // There is nowhere to paste for codex: the code route refuses.
    let refused = app
        .oneshot(authed(request(
            "POST",
            &format!("/api/agent-auth/{id}/code"),
            Some(serde_json::json!({"code": "TEST-CODE1"})),
        )))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(refused).await["code"], "input_not_required");
}

#[tokio::test]
async fn an_unknown_provider_is_a_bad_request() {
    let (state, _, _) = state().await;
    let app = router(state);
    let response = app
        .oneshot(authed(request(
            "POST",
            "/api/agent-auth/start",
            Some(serde_json::json!({"provider": "gemini"})),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(response).await["code"], "bad_request");
}
