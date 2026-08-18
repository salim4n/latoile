//! Route tests: the router driven through `tower::oneshot` against an
//! ephemeral SQLite store, a stub agent channel, and a stub GitHub client —
//! no process, no network beyond loopback, no real token.

use crate::router;
use crate::state::{AgentSlot, AppState, GitHubSlot};
use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use latoile_app::store::Store;
use latoile_core::ids::{ArchitectureSessionId, ProjectId, RunId};
use latoile_core::ports::{
    AgentChannel, ArchitectReply, ArchitecturePackageReply, ArchitecturePackageRequest,
    ArchitectureSessionStore, ManagerReply, PortResult, RepoInfo,
};
use latoile_core::{
    ARCHITECT_SKILL_NAME, ArchitectureOperatingMode, ArchitecturePackageEvidence,
    ArchitecturePackageValidation, ArchitectureValidationFinding, ArchitectureVisualScenario, Run,
    SpecProvenance, SpecVersion,
};
use latoile_preview::{Supervisor, SupervisorConfig};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

pub const TOKEN: &str = "test-token";

pub async fn attach_test_spec_provenance(store: &Store, spec: &mut SpecVersion) {
    let session_id =
        ArchitectureSessionId::new(format!("architecture-{}", spec.id.as_str())).unwrap();
    let mut session =
        latoile_core::ArchitectureSession::new(session_id.clone(), spec.project_id.clone());
    session.cancel().unwrap();
    ArchitectureSessionStore::save(store, &session)
        .await
        .unwrap();
    spec.attach_provenance(SpecProvenance {
        architecture_session_id: session_id,
        skill_name: ARCHITECT_SKILL_NAME.into(),
        skill_digest: "a".repeat(64),
        operating_mode: ArchitectureOperatingMode::Greenfield,
        package_digest: "b".repeat(64),
        manifest_digest: "c".repeat(64),
        package_commit_sha: "1".repeat(40),
        package_tree_sha: "2".repeat(40),
    })
    .unwrap();
}

pub async fn approve_test_spec(store: &Store, spec: &mut SpecVersion) {
    attach_test_spec_provenance(store, spec).await;
    let provenance = spec.provenance.as_ref().unwrap();
    let verification = ArchitecturePackageValidation {
        valid: true,
        package_digest: provenance.package_digest.clone(),
        manifest_digest: provenance.manifest_digest.clone(),
        commit_sha: provenance.package_commit_sha.clone(),
        tree_sha: provenance.package_tree_sha.clone(),
        file_count: 16,
        gallery_path: "gallery.html".into(),
        scenarios: vec![ArchitectureVisualScenario {
            comparison_id: "home-default-fr-mobile".into(),
            screen: "home".into(),
            state: "default".into(),
            locale: "fr-FR".into(),
            viewport_width: 390,
            viewport_height: 844,
            device_scale_factor_milli: 1000,
            mockup: "mockups/home-default-fr-mobile.html".into(),
        }],
        findings: Vec::new(),
    };
    spec.approve(&verification).unwrap();
}

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
    pub architecture_messages: Arc<Mutex<Vec<String>>>,
    pub architecture_replies: Arc<Mutex<std::collections::VecDeque<String>>>,
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
            architecture_messages: Arc::new(Mutex::new(Vec::new())),
            architecture_replies: Arc::new(Mutex::new(std::collections::VecDeque::from([
                "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"question\",\"phase\":\"domain_discovery\",\"message\":\"Quel problème doit disparaître pour l'utilisateur ?\"}\n```".into(),
                "```latoile-architecture\n{\"schema_version\":1,\"kind\":\"ready_to_draft\",\"phase\":\"ready_to_draft\",\"message\":\"Les décisions sont suffisantes pour produire le paquet.\"}\n```".into(),
            ]))),
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
    async fn start_architecture(
        &self,
        _project: &ProjectId,
        session: &ArchitectureSessionId,
        brief: &str,
    ) -> PortResult<ArchitectReply> {
        self.architecture_messages
            .lock()
            .unwrap()
            .push(format!("brief:{brief}"));
        Ok(ArchitectReply {
            content: self
                .architecture_replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| "missing scripted Architect reply".into()),
            acp_session_id: format!("acp-architecture:{}", session.as_str()),
            skill_name: ARCHITECT_SKILL_NAME.into(),
            skill_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            operating_mode: ArchitectureOperatingMode::Greenfield,
        })
    }
    async fn continue_architecture(
        &self,
        _project: &ProjectId,
        session: &ArchitectureSessionId,
        answer: &str,
    ) -> PortResult<ArchitectReply> {
        self.architecture_messages
            .lock()
            .unwrap()
            .push(format!("answer:{answer}"));
        Ok(ArchitectReply {
            content: self
                .architecture_replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| "missing scripted Architect reply".into()),
            acp_session_id: format!("acp-architecture:{}", session.as_str()),
            skill_name: ARCHITECT_SKILL_NAME.into(),
            skill_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            operating_mode: ArchitectureOperatingMode::Greenfield,
        })
    }
    async fn generate_architecture_package(
        &self,
        _project: &ProjectId,
        _session: &ArchitectureSessionId,
        request: &ArchitecturePackageRequest,
    ) -> PortResult<ArchitecturePackageReply> {
        Ok(ArchitecturePackageReply {
            evidence: ArchitecturePackageEvidence {
                design_dir: request.design_dir.clone(),
                base_sha: "1111111111111111111111111111111111111111".into(),
                head_sha: "2222222222222222222222222222222222222222".into(),
                tree_sha: "3333333333333333333333333333333333333333".into(),
                package_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .into(),
                manifest_digest: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .into(),
                changed_files: vec![format!("{}architecture-spec.md", request.design_dir)],
                diff_stat: "14 files changed".into(),
            },
            summary: "Package complete".into(),
        })
    }
    async fn verify_architecture_package(
        &self,
        _project: &ProjectId,
        spec: &SpecVersion,
    ) -> PortResult<ArchitecturePackageValidation> {
        let provenance = spec.provenance.as_ref().ok_or_else(|| {
            latoile_core::ports::PortError("draft has no package provenance".into())
        })?;
        Ok(ArchitecturePackageValidation {
            valid: true,
            package_digest: provenance.package_digest.clone(),
            manifest_digest: provenance.manifest_digest.clone(),
            commit_sha: provenance.package_commit_sha.clone(),
            tree_sha: provenance.package_tree_sha.clone(),
            file_count: 16,
            gallery_path: "gallery.html".into(),
            scenarios: vec![ArchitectureVisualScenario {
                comparison_id: "home-default-fr-mobile".into(),
                screen: "home".into(),
                state: "default".into(),
                locale: "fr-FR".into(),
                viewport_width: 390,
                viewport_height: 844,
                device_scale_factor_milli: 1000,
                mockup: "mockups/home-default-fr-mobile.html".into(),
            }],
            findings: vec![ArchitectureValidationFinding {
                code: "stub_verified".into(),
                message: "Stub package verified.".into(),
            }],
        })
    }
    async fn read_architecture_artifact(
        &self,
        _project: &ProjectId,
        _spec: &SpecVersion,
        relative_path: &str,
    ) -> PortResult<String> {
        Ok(format!(
            "<!doctype html><html><body>stub artifact {relative_path}</body></html>"
        ))
    }
    async fn cancel_architecture(&self, session: &ArchitectureSessionId) -> PortResult<()> {
        self.architecture_messages
            .lock()
            .unwrap()
            .push(format!("cancel:{}", session.as_str()));
        Ok(())
    }
    async fn start_run(&self, _project: &ProjectId, r: &Run, prompt: &str) -> PortResult<String> {
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
            let login_claude = "printf 'https://claude.com/oauth/authorize?test=1\\n'; read line; [ \"$line\" = good ]";
            let login_codex = "printf 'Go to https://auth.openai.com/codex/device and enter TEST-CODE1\\n'; sleep 60";
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
