//! Route tests: the router driven through `tower::oneshot` against an
//! ephemeral SQLite store, a stub agent channel, and a stub GitHub client —
//! no process, no network beyond loopback, no real token.

use crate::router;
use crate::state::{AgentSlot, AppState, GitHubSlot};
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
#[derive(Clone)]
pub struct StubAgents {
    pub manager_messages: Arc<Mutex<Vec<String>>>,
    /// The Manager's next answer — scriptable per test.
    pub manager_reply: Arc<Mutex<String>>,
    /// Scripted supervision answers, keyed by run id.
    pub run_states: Arc<Mutex<std::collections::HashMap<String, latoile_agents::RunState>>>,
    /// Role and prompt for every spawned run, used to prove review context.
    pub run_prompts: Arc<Mutex<Vec<(String, String)>>>,
    /// Live permission request ids keyed by run, plus forwarded decisions.
    pub live_permissions: Arc<Mutex<std::collections::HashMap<String, String>>>,
    pub permission_decisions: Arc<Mutex<Vec<(String, String, bool)>>>,
}

impl Default for StubAgents {
    fn default() -> Self {
        Self {
            manager_messages: Arc::new(Mutex::new(Vec::new())),
            manager_reply: Arc::new(Mutex::new("Bien reçu, je m'en occupe.".into())),
            run_states: Arc::new(Mutex::new(std::collections::HashMap::new())),
            run_prompts: Arc::new(Mutex::new(Vec::new())),
            live_permissions: Arc::new(Mutex::new(std::collections::HashMap::new())),
            permission_decisions: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl AgentChannel for StubAgents {
    async fn tell_manager(&self, _p: &ProjectId, message: &str) -> PortResult<ManagerReply> {
        self.manager_messages
            .lock()
            .unwrap()
            .push(message.to_string());
        Ok(ManagerReply {
            content: self.manager_reply.lock().unwrap().clone(),
            actions: None,
        })
    }
    async fn start_run(&self, r: &Run, prompt: &str) -> PortResult<String> {
        // Registered as running so the supervision driver can be scripted:
        // tests flip the entry to Done/Failed when the "agent" is done.
        self.run_states
            .lock()
            .unwrap()
            .insert(r.id.as_str().to_string(), latoile_agents::RunState::Running);
        self.run_prompts
            .lock()
            .unwrap()
            .push((r.role_id.as_str().to_string(), prompt.to_string()));
        Ok("acp-stub".into())
    }
    async fn cancel_run(&self, _r: &RunId) -> PortResult<()> {
        Ok(())
    }
    async fn resolve_permission(
        &self,
        run: &RunId,
        request_id: &str,
        granted: bool,
    ) -> PortResult<()> {
        let mut pending = self.live_permissions.lock().unwrap();
        if pending.get(run.as_str()).map(String::as_str) != Some(request_id) {
            return Err(latoile_core::ports::PortError(
                "pending ACP permission request was not found".into(),
            ));
        }
        pending.remove(run.as_str());
        self.permission_decisions.lock().unwrap().push((
            run.as_str().to_string(),
            request_id.to_string(),
            granted,
        ));
        self.run_states
            .lock()
            .unwrap()
            .insert(run.as_str().to_string(), latoile_agents::RunState::Running);
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
            private: true,
        }]),
        previews: Supervisor::new(SupervisorConfig {
            base_port: 26100,
            // The driver Ensures a preview after a finished run; in tests
            // the dev command never listens, so keep the failure cheap.
            readiness: std::time::Duration::from_millis(200),
            ..SupervisorConfig::default()
        }),
        proxy_http: reqwest::Client::new(),
        agent_auth: {
            let cmd = |script: &str| latoile_agents::AgentCommand {
                program: "sh".into(),
                args: vec!["-c".into(), script.into()],
                env: Vec::new(),
            };
            let login_claude =
                "printf 'https://claude.com/oauth/authorize?test=1\\n'; read line; [ \"$line\" = good ]";
            let login_codex =
                "printf 'Go to https://auth.openai.com/codex/device and enter TEST-CODE1\\n'; sleep 60";
            latoile_agents::AgentAuthManager::new(latoile_agents::DEFAULT_TTL)
                .with_commands(
                    latoile_agents::AuthProvider::Claude,
                    latoile_agents::ProviderCommands {
                        login: cmd(login_claude),
                        status: cmd("printf '{\"loggedIn\": true, \"email\":\"moi@example.com\"}'"),
                        logout: cmd("true"),
                    },
                )
                .with_commands(
                    latoile_agents::AuthProvider::Codex,
                    latoile_agents::ProviderCommands {
                        login: cmd(login_codex),
                        status: cmd("echo 'Not logged in'; exit 1"),
                        logout: cmd("true"),
                    },
                )
        },
        routing: latoile_agents::SharedRouting::default(),
        decision_lock: Arc::new(tokio::sync::Mutex::new(())),
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
                "work_branch": "work"
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
mod agent_auth;
mod driver;
mod flows;
mod http;
mod settings;
