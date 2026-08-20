use super::*;

#[tokio::test]
async fn english_architect_turn_is_persisted_without_a_french_manager_wrapper() {
    let (state, _, agents) = state().await;
    *agents.architecture_replies.lock().unwrap() = std::collections::VecDeque::from([
        "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"question\",\"phase\":\"domain_discovery\",\"message\":\"Who must approve the reference mockup?\"}\n```".into(),
    ]);
    let app = router(state);
    let project = create_project(&app).await;

    let response = app
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Build a visual QA workspace in English.",
                "intent": "architecture_brief"
            })),
        )))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(
        body["reply"]["content"],
        "Who must approve the reference mockup?"
    );
    let actions: serde_json::Value =
        serde_json::from_str(body["reply"]["actions"].as_str().unwrap()).unwrap();
    assert_eq!(actions[0]["type"], "architecture");
    assert_eq!(actions[0]["kind"], "question");
    assert!(actions[0].get("title").is_none());
}
