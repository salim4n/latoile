//! Settings routes: routing round-trip and validation, auth status shape,
//! disconnect.

use super::*;
use axum::http::StatusCode;
use tower::ServiceExt;

#[tokio::test]
async fn routing_defaults_round_trip_and_persist() {
    let (state, _, _) = state().await;
    let app = router(state);

    let response = app
        .clone()
        .oneshot(authed(request("GET", "/api/settings/routing", None)))
        .await
        .unwrap();
    let body = body_json(response).await;
    assert_eq!(body["manager"], "claude");
    assert_eq!(body["reviewer"], "claude");

    let put = app
        .clone()
        .oneshot(authed(request(
            "PUT",
            "/api/settings/routing",
            Some(serde_json::json!({
                "manager": "claude",
                "architect": "claude",
                "backend": "codex",
                "frontend": "codex",
                "reviewer": "claude"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    assert_eq!(body_json(put).await["backend"], "codex");

    let again = app
        .oneshot(authed(request("GET", "/api/settings/routing", None)))
        .await
        .unwrap();
    assert_eq!(body_json(again).await["frontend"], "codex");
}

#[tokio::test]
async fn an_unknown_provider_in_routing_is_a_domain_error() {
    let (state, _, _) = state().await;
    let app = router(state);
    let response = app
        .oneshot(authed(request(
            "PUT",
            "/api/settings/routing",
            Some(serde_json::json!({
                "manager": "claude",
                "architect": "claude",
                "backend": "gemini",
                "frontend": "claude",
                "reviewer": "claude"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(response).await["code"], "domain_refused");
}

#[tokio::test]
async fn the_status_endpoint_reports_both_providers() {
    let (state, _, _) = state().await;
    let app = router(state);

    let response = app
        .oneshot(authed(request("GET", "/api/agent-auth/status", None)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    // The fixture: claude logged in (with its email), codex not.
    assert_eq!(body["claude"]["authenticated"], true);
    assert_eq!(body["claude"]["detail"], "moi@example.com");
    assert_eq!(body["codex"]["authenticated"], false);
}

#[tokio::test]
async fn disconnect_calls_logout_and_reports_the_status_after() {
    let (state, _, _) = state().await;
    let app = router(state);

    let response = app
        .clone()
        .oneshot(authed(request(
            "POST",
            "/api/agent-auth/disconnect",
            Some(serde_json::json!({"provider": "codex"})),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["authenticated"], false);

    let bad = app
        .oneshot(authed(request(
            "POST",
            "/api/agent-auth/disconnect",
            Some(serde_json::json!({"provider": "mistral"})),
        )))
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}
