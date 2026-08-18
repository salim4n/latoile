//! App state and assembly. `build` wires the concrete adapters — the only
//! place outside the CLI that knows all of them at once. Handlers see
//! [`AppState`]: the store, the agent channel, the preview supervisor, the
//! GitHub client, the token.
//!
//! **Token choice (D9).** `ServerConfig.token` wins; else `LATOILE_TOKEN`
//! from the environment; else one is generated and returned by [`build`] so
//! the CLI can print it once at startup. There is no user database to look
//! anything up in — a single random bearer token is the whole mechanism.
//!
//! **Why the slots are enums.** The core ports are RPITIT traits — not
//! object-safe, and a router generic over them could not promise `Send`
//! handler futures. An enum over the concrete adapter plus a test stub
//! keeps the router concrete, `Send`-checked, and fakeable without touching
//! the app's type safety.

use crate::dirs::StoreDirs;
use latoile_agents::{AcpChannel, AgentTimeouts, ChannelConfig, ProcessConnector, SharedRouting};
use latoile_app::store::Store;
use latoile_core::ids::{ProjectId, RunId};
use latoile_core::ports::{
    AgentChannel, GitHubClient, ManagerReply, PortResult, ProvisionWorkspaceInput,
    ProvisionedWorkspace, RepoInfo, WorkspaceProvisioner,
};
use latoile_core::Run;
use latoile_github::{GitHub, GitHubConfig};
use latoile_preview::Supervisor;
use latoile_vault::Vault;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const TOKEN_ENV: &str = "LATOILE_TOKEN";

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Explicit token; falls back to `LATOILE_TOKEN`, then to a generated
    /// one printed at startup.
    pub token: Option<String>,
    /// Agent sessions run with this as their root directory.
    pub workspace: PathBuf,
    /// Role skill preambles (`<dir>/<skill>/SKILL.md`).
    pub skills_dir: PathBuf,
    /// Where the vault's `master.key` lives (created 0600 on first run).
    pub config_home: PathBuf,
    /// GitHub API base — `None` means https://api.github.com.
    pub github_api_base: Option<String>,
}

/// What failed while wiring the server.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("store: {0}")]
    Store(#[from] latoile_app::store::StoreError),
    #[error("vault: {0}")]
    Vault(#[from] latoile_vault::VaultError),
}

/// The agent channel, concrete. `Stub` exists only in tests.
#[derive(Clone)]
pub enum AgentSlot {
    Real(Arc<AcpChannel<ProcessConnector, StoreDirs, SharedRouting>>),
    #[cfg(test)]
    Stub(crate::tests::StubAgents),
}

impl AgentChannel for AgentSlot {
    async fn tell_manager(&self, project: &ProjectId, message: &str) -> PortResult<ManagerReply> {
        match self {
            Self::Real(channel) => channel.tell_manager(project, message).await,
            #[cfg(test)]
            Self::Stub(stub) => stub.tell_manager(project, message).await,
        }
    }
    async fn start_run(&self, run: &Run, prompt: &str) -> PortResult<String> {
        match self {
            Self::Real(channel) => channel.start_run(run, prompt).await,
            #[cfg(test)]
            Self::Stub(stub) => stub.start_run(run, prompt).await,
        }
    }
    async fn cancel_run(&self, run: &RunId) -> PortResult<()> {
        match self {
            Self::Real(channel) => channel.cancel_run(run).await,
            #[cfg(test)]
            Self::Stub(stub) => stub.cancel_run(run).await,
        }
    }
}

/// The GitHub client, concrete. `Stub` exists only in tests.
#[derive(Clone)]
pub enum GitHubSlot {
    Real(GitHub<Vault>),
    #[cfg(test)]
    Stub(Vec<RepoInfo>),
}

impl GitHubClient for GitHubSlot {
    async fn list_repos(&self) -> PortResult<Vec<RepoInfo>> {
        match self {
            Self::Real(github) => github.list_repos().await,
            #[cfg(test)]
            Self::Stub(repos) => Ok(repos.clone()),
        }
    }
    async fn open_pull_request(&self, repo: &str, head: &str, base: &str) -> PortResult<String> {
        match self {
            Self::Real(github) => github.open_pull_request(repo, head, base).await,
            #[cfg(test)]
            Self::Stub(_) => Ok("https://github.com/stub/pr/1".into()),
        }
    }
}

impl WorkspaceProvisioner for GitHubSlot {
    async fn provision(&self, input: &ProvisionWorkspaceInput) -> PortResult<ProvisionedWorkspace> {
        match self {
            Self::Real(github) => github.provision(input).await,
            #[cfg(test)]
            Self::Stub(_) => Ok(ProvisionedWorkspace {
                default_branch: "main".into(),
                work_branch: input.work_branch.clone(),
                local_path: format!("/srv/latoile/{}", input.slug),
                dev_command: input
                    .dev_command
                    .clone()
                    .unwrap_or_else(|| "pnpm dev -- --port $PORT".into()),
            }),
        }
    }
}

impl AgentSlot {
    /// Drop persistent manager sessions (routing changed). The next message
    /// respawns under the new provider; runs are ephemeral and unaffected.
    pub async fn evict_managers(&self) {
        match self {
            Self::Real(channel) => channel.evict_managers().await,
            #[cfg(test)]
            Self::Stub(_) => {}
        }
    }

    /// What the channel recorded for a run — the supervision driver's
    /// window into the agent processes. `None` means the channel never saw
    /// this run (a restart loses the registry: the run is lost).
    pub async fn run_state(&self, run: &RunId) -> Option<latoile_agents::RunState> {
        match self {
            Self::Real(channel) => channel.run_state(run).await,
            #[cfg(test)]
            Self::Stub(stub) => stub.run_states.lock().unwrap().get(run.as_str()).cloned(),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub agents: AgentSlot,
    pub github: GitHubSlot,
    pub previews: Supervisor,
    /// For the preview reverse proxy — separate from the GitHub client so a
    /// proxy failure never touches API state.
    pub proxy_http: reqwest::Client,
    /// Click-to-login sessions for the agent runtime.
    pub agent_auth: latoile_agents::AgentAuthManager,
    /// The live role→provider map the channel reads; refreshed on PUT.
    pub routing: SharedRouting,
    /// One process-wide decision critical section. LaToile is single-user;
    /// serializing this tiny write path makes HTTP retries/concurrency
    /// exactly-once before they reach agent spawning.
    pub decision_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) token: Arc<str>,
}

impl AppState {
    pub fn token(&self) -> &str {
        &self.token
    }
}

/// Resolve the token: config, then environment, then generate.
fn resolve_token(config: &ServerConfig) -> (String, &'static str) {
    if let Some(token) = &config.token {
        return (token.clone(), "config");
    }
    if let Ok(token) = std::env::var(TOKEN_ENV) {
        if !token.trim().is_empty() {
            return (token, TOKEN_ENV);
        }
    }
    (ulid::Ulid::new().to_string(), "generated")
}

/// Wire every adapter and return the router plus the token in effect — the
/// CLI prints the token when it was generated.
/// The router, the token in effect and its source, and the supervision
/// driver's handle (abort it on shutdown).
pub async fn build(
    config: &ServerConfig,
    db_path: &Path,
) -> Result<
    (
        axum::Router,
        String,
        &'static str,
        tokio::task::JoinHandle<()>,
    ),
    BuildError,
> {
    let store = Store::open(db_path).await?;
    let (token, token_source) = resolve_token(config);

    let (root_key, _source) = latoile_vault::load_root_key(&config.config_home)?;
    let vault = Vault::open(db_path, root_key).await?;
    let github = GitHub::new(
        GitHubConfig {
            api_base: config
                .github_api_base
                .clone()
                .unwrap_or_else(|| GitHubConfig::default().api_base),
            workspace_root: config.workspace.clone(),
            ..GitHubConfig::default()
        },
        vault,
        GitHub::<Vault>::default_http(),
    );

    let timeouts = AgentTimeouts::default();
    // Seed the live routing from the settings table.
    let routing = SharedRouting::default();
    let stored = latoile_app::use_cases::Routing::new(store.clone())
        .get()
        .await
        .map_err(|e| {
            BuildError::Store(latoile_app::store::StoreError::CorruptRow(e.to_string()))
        })?;
    routing.set_all(stored.into_iter().map(|r| (r.role, r.provider)).collect());

    let agents = AcpChannel::new(
        ChannelConfig {
            skills_dir: config.skills_dir.clone(),
            commands: std::collections::HashMap::new(),
            ..ChannelConfig::default()
        },
        ProcessConnector {
            handshake: timeouts.handshake,
        },
        StoreDirs::new(store.clone(), config.workspace.clone()),
        routing.clone(),
    );

    let state = AppState {
        store,
        agents: AgentSlot::Real(Arc::new(agents)),
        github: GitHubSlot::Real(github),
        previews: Supervisor::default(),
        proxy_http: reqwest::Client::new(),
        agent_auth: latoile_agents::AgentAuthManager::production(),
        routing,
        decision_lock: Arc::new(tokio::sync::Mutex::new(())),
        token: Arc::from(token.as_str()),
    };
    let driver = crate::driver::spawn(state.clone());
    Ok((crate::routes::router(state), token, token_source, driver))
}
