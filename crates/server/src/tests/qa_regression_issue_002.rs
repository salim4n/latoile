use super::*;

#[tokio::test]
async fn missing_preview_is_a_successful_empty_read_model() {
    let (state, _, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    let response = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/preview"),
            None,
        )))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await, serde_json::Value::Null);
}
