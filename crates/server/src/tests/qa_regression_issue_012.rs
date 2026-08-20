use super::*;

#[tokio::test]
async fn architecture_brief_persists_the_owner_selected_package_locale() {
    let (state, _, _) = state().await;
    let app = router(state);
    let project = create_project(&app).await;

    let started = app
        .clone()
        .oneshot(authed(request(
            "POST",
            &format!("/api/projects/{project}/messages"),
            Some(serde_json::json!({
                "content": "Concevoir une page de validation locale.",
                "intent": "architecture_brief",
                "locale": "fr-FR"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);

    let architecture = app
        .oneshot(authed(request(
            "GET",
            &format!("/api/projects/{project}/architecture"),
            None,
        )))
        .await
        .unwrap();
    let architecture = body_json(architecture).await;
    assert_eq!(architecture["requested_locale"], "fr-FR");
}
