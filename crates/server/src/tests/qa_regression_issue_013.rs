use super::*;

#[tokio::test]
async fn architecture_session_persists_the_original_owner_brief() {
    let (state, store, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;
    let brief = "Exactly one mobile P0 scenario; no desktop or alternate states.";

    let started = app
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": brief,
                "intent": "architecture_brief",
                "locale": "en-US"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);

    let project_id = ProjectId::new(project).unwrap();
    let session = store.latest_for_project(&project_id).await.unwrap().unwrap();
    assert_eq!(session.brief, brief);
}
