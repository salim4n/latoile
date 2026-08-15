//! Route tests: the router driven through `tower::oneshot` against an
//! ephemeral SQLite store, a stub agent channel, and a stub GitHub client —
//! no process, no network beyond loopback, no real token.

use crate::state::{AgentSlot, AppState, GitHubSlot};
use crate::router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use latoile_app::store::Store;
use latoile_core::ids::{ProjectId, RunId};
use latoile_core::ports::{AgentChannel, ManagerReply, PortResult, RepoInfo};
use latoile_core::Run;
use latoile_preview::{Supervisor, SupervisorConfig};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

pub const TOKEN: &str = "test-token";

/// The scripted agent channel: the Manager answers with a canned reply,
/// runs get a canned session handle. Records what it was told.
#[derive(Clone, Default)]
pub struct StubAgents {
    pub manager_messages: Arc<Mutex<Vec<String>>>,
}

impl AgentChannel for StubAgents {
    async fn tell_manager(&self, _p: &ProjectId, message: &str) -> PortResult<ManagerReply> {
        self.manager_messages
            .lock()
            .unwrap()
            .push(message.to_string());
        Ok(ManagerReply {
            content: "Bien reçu, je m'en occupe.".into(),
            actions: None,
        })
    }
    async fn start_run(&self, _r: &Run, _p: &str) -> PortResult<String> {
        Ok("acp-stub".into())
    }
    async fn cancel_run(&self, _r: &RunId) -> PortResult<()> {
        Ok(())
    }
}

pub async fn state() -> (AppState, Store, StubAgents) {
    let store = Store::open_ephemeral().await.unwrap();
    let agents = StubAgents::default();
    let state = AppState {
        store: store.clone(),
        agents: AgentSlot::Stub(agents.clone()),
        github: GitHubSlot::Stub(vec![RepoInfo {
            full_name: "salim4n/mon-app".into(),
            description: None,
        }]),
        previews: Supervisor::new(SupervisorConfig {
            base_port: 26100,
            ..SupervisorConfig::default()
        }),
        proxy_http: reqwest::Client::new(),
        token: Arc::from(TOKEN),
    };
    (state, store, agents)
}

pub fn authed(request: Request) -> Request {
    let (mut parts, body) = request.into_parts();
    parts
        .headers
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    Request::from_parts(parts, body)
}

pub fn request(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(match body {
            Some(json) => Body::from(json.to_string()),
            None => Body::empty(),
        })
        .unwrap()
}

/// Create a project through the API; returns its id.
pub async fn create_project(app: &axum::Router) -> String {
    let response = app
        .clone()
        .oneshot(authed(request(
            "POST",
            "/api/projects",
            Some(serde_json::json!({
                "name": "Mon App",
                "slug": "mon-app",
                "github_repo": "salim4n/mon-app",
                "work_branch": "work",
                "local_path": "/srv/latoile/mon-app",
                "dev_command": "pnpm dev --port $PORT"
            })),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    body["id"].as_str().unwrap().to_string()
}

pub async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}
mod flows;
mod http;
