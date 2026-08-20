//! The `AgentChannel` port. Two lifecycles, per the port's contract:
//!
//! - **Manager**: one persistent ACP session per project, spawned on first
//!   message, resumed after. The role's skill preamble heads the first
//!   prompt only.
//! - **Architect discovery**: one persistent ACP session per architecture
//!   session. It is read-only and emits a strict question/ready contract.
//! - **Runs**: one fresh process per run; the prompt turn runs on a
//!   background task and the process dies with it. `cancel_run` aborts the
//!   task, which drops the connection, which kills the process group.
//!
//! Auth note: the spawned CLIs carry their own credentials from the user's
//! machine. This crate never sees, stores, or forwards an agent API key —
//! the D9 token rule is about LaToile's own HTTP API.
//!
//! What the port does NOT carry: run completion. `start_run` returns the
//! session handle and the turn continues in the background; the outcome is
//! recorded here (`run_state`) for the supervision driver to poll and apply
//! through the app layer. The poll boundary is the deliberate V1 contract.

use crate::architecture_package;
use crate::config::{AgentCommand, ChannelConfig};
use crate::error::AgentError;
use crate::permissions::PermissionBroker;
use crate::preamble::Preambles;
use crate::transport::{Connection, Connector, PermissionContext};
use crate::updates::{AgentUpdate, RunOutcome};
use latoile_core::ids::{ArchitectureSessionId, ProjectId, RunId};
use latoile_core::ports::{
    AgentChannel, ArchitectReply, ArchitecturePackageReply, ArchitecturePackageRequest,
    ManagerReply, PortResult,
};
use latoile_core::{ArchitectureOperatingMode, ArchitecturePhase, Run};
use latoile_core::{ArchitecturePackageValidation, SpecVersion};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

/// Which binary a role runs under. Sync and infallible: the server keeps a
/// shared map refreshed from the settings table; the agents crate never
/// sees the database.
pub trait RoutingSource: Send + Sync {
    fn command_for(&self, role: &str) -> AgentCommand;
}

/// The static map from configuration — every role falls back to its
/// default command.
impl RoutingSource for ChannelConfig {
    fn command_for(&self, role: &str) -> AgentCommand {
        ChannelConfig::command_for(self, role).clone()
    }
}

/// The routing map the server refreshes from the settings table. Reads are
/// a lock away; writes replace the map wholesale (five rows, no deltas).
#[derive(Clone, Default)]
pub struct SharedRouting {
    inner: std::sync::Arc<std::sync::RwLock<HashMap<String, String>>>,
}

impl SharedRouting {
    pub fn set_all(&self, entries: Vec<(String, String)>) {
        *self.inner.write().expect("routing poisoned") = entries.into_iter().collect();
    }
}

impl RoutingSource for SharedRouting {
    fn command_for(&self, role: &str) -> AgentCommand {
        let provider = self
            .inner
            .read()
            .expect("routing poisoned")
            .get(role)
            .cloned()
            .unwrap_or_else(|| "claude".into());
        let binary = match provider.as_str() {
            "codex" => "codex-acp",
            _ => "claude-agent-acp",
        };
        AgentCommand::new(binary)
    }
}

/// Where sessions work: the project's `local_path` — the checkout the code
/// lives in. Resolution needs the store, so the trait is async (desugared
/// `Send` futures: the channel's callers are axum handlers). `None` means
/// unknown project; a nonexistent directory is refused before any spawn.
pub trait ProjectDirs: Send + Sync {
    fn manager_dir<'a>(
        &'a self,
        project: &'a ProjectId,
    ) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'a;
    fn run_dir<'a>(
        &'a self,
        project: &'a ProjectId,
        run: &'a Run,
    ) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'a;
}

/// Every session in one directory — the tests' fixture.
pub struct RootDirs(pub PathBuf);

#[allow(clippy::manual_async_fn)] // the trait needs the explicit `+ Send`
impl ProjectDirs for RootDirs {
    fn manager_dir<'a>(
        &'a self,
        _project: &'a ProjectId,
    ) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'a {
        async move { Some(self.0.clone()) }
    }
    fn run_dir<'a>(
        &'a self,
        _project: &'a ProjectId,
        _run: &'a Run,
    ) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'a {
        async move { Some(self.0.clone()) }
    }
}

/// Refuse a workspace that doesn't exist BEFORE anything spawns — a bad cwd
/// surfaces as a clear error, never as a 30-second timeout.
fn checked_dir(dir: PathBuf) -> Result<PathBuf, AgentError> {
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(AgentError::NoWorkspace(format!(
            "the project directory does not exist: {}",
            dir.display()
        )))
    }
}

/// Where a background run stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    Running,
    /// The ACP responder is parked until the exact request is decided.
    Blocked(latoile_core::ports::PermissionRequest),
    /// The wait budget elapsed; ACP received a refusal and the application
    /// must close the persisted approval before normal supervision resumes.
    PermissionExpired(latoile_core::ports::PermissionRequest),
    Done(RunReport),
    /// Transport or timeout failure; the message is for logs, not clients.
    Failed(String),
}

/// Sanitized terminal evidence retained for orchestration and review. Raw
/// prompts, thought chunks and tool inputs never enter this structure.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RunReport {
    pub outcome: RunOutcome,
    pub summary: String,
    pub activity: Vec<String>,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub commits: Vec<CommitEvidence>,
    pub changed_files: Vec<ChangedFileEvidence>,
    pub diff_stat: String,
}

impl RunReport {
    /// Minimal report for adapters and tests that have no repository
    /// evidence to attach. Production ACP runs use the richer collector.
    pub fn terminal(outcome: RunOutcome, summary: impl Into<String>) -> Self {
        Self {
            outcome,
            summary: summary.into(),
            activity: Vec::new(),
            base_sha: None,
            head_sha: None,
            commits: Vec::new(),
            changed_files: Vec::new(),
            diff_stat: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CommitEvidence {
    pub sha: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChangedFileEvidence {
    pub status: String,
    pub path: String,
}

struct RunEntry {
    abort: AbortHandle,
    state: Arc<StdMutex<RunState>>,
}

struct ManagerEntry<C> {
    conn: C,
    greeted: bool,
}

/// One slot per project: the per-entry mutex lets two projects' managers
/// answer concurrently while prompts on one manager stay serial.
type ManagerSlot<C> = Arc<Mutex<ManagerEntry<C>>>;

struct ArchitectEntry<C> {
    conn: C,
    skill_name: String,
    skill_digest: String,
    operating_mode: ArchitectureOperatingMode,
}

type ArchitectSlot<C> = Arc<Mutex<ArchitectEntry<C>>>;

pub struct AcpChannel<C: Connector, D: ProjectDirs, R: RoutingSource> {
    config: ChannelConfig,
    connector: C,
    dirs: D,
    routing: R,
    preambles: Preambles,
    managers: Mutex<HashMap<String, ManagerSlot<C::Conn>>>,
    architects: Mutex<HashMap<String, ArchitectSlot<C::Conn>>>,
    runs: Mutex<HashMap<String, RunEntry>>,
    permissions: PermissionBroker,
}

impl<C: Connector, D: ProjectDirs, R: RoutingSource> AcpChannel<C, D, R> {
    pub fn new(config: ChannelConfig, connector: C, dirs: D, routing: R) -> Self {
        let preambles = Preambles::new(config.skills_dir.clone());
        Self {
            config,
            connector,
            dirs,
            routing,
            preambles,
            managers: Mutex::new(HashMap::new()),
            architects: Mutex::new(HashMap::new()),
            runs: Mutex::new(HashMap::new()),
            permissions: PermissionBroker::default(),
        }
    }

    /// Drop every persistent manager session; the next message respawns
    /// under the current routing. Called when routing changes — a running
    /// session keeps the provider it started with until then.
    pub async fn evict_managers(&self) {
        self.managers.lock().await.clear();
    }

    /// Where a run stands, for the app layer polling from its own loop.
    /// `None` = unknown run.
    pub async fn run_state(&self, run: &RunId) -> Option<RunState> {
        let runs = self.runs.lock().await;
        let state = runs
            .get(run.as_str())
            .map(|entry| entry.state.lock().expect("run state poisoned").clone())?;
        if let Some(request) = self.permissions.expired_for_run(run) {
            return Some(RunState::PermissionExpired(request));
        }
        if state == RunState::Running {
            if let Some(request) = self.permissions.pending_for_run(run) {
                return Some(RunState::Blocked(request));
            }
        }
        Some(state)
    }

    /// Remove a timeout marker only after its persisted approval has been
    /// closed successfully by the supervision driver.
    pub fn acknowledge_permission_expiry(&self, run: &RunId, request_id: &str) {
        self.permissions.acknowledge_expiry(run, request_id);
    }

    async fn manager_for(&self, project: &ProjectId) -> Result<ManagerSlot<C::Conn>, AgentError> {
        let mut managers = self.managers.lock().await;
        if let Some(entry) = managers.get(project.as_str()) {
            return Ok(entry.clone());
        }
        let dir = self
            .dirs
            .manager_dir(project)
            .await
            .ok_or_else(|| AgentError::NoWorkspace(format!("project {}", project.as_str())))
            .and_then(checked_dir)?;
        let command = self.routing.command_for("manager");
        let mut conn = self
            .connector
            .connect(
                &command,
                &dir,
                PermissionContext {
                    role_id: "manager".into(),
                    run_id: None,
                    write_root: None,
                    broker: self.permissions.clone(),
                    timeout: self.config.timeouts.permission,
                },
            )
            .await?;
        conn.new_session(&dir).await?;
        let entry = Arc::new(Mutex::new(ManagerEntry {
            conn,
            greeted: false,
        }));
        managers.insert(project.as_str().to_string(), entry.clone());
        Ok(entry)
    }

    async fn architecture_prompt(
        &self,
        entry: &ArchitectSlot<C::Conn>,
        session: &ArchitectureSessionId,
        prompt: &str,
    ) -> Result<ArchitectReply, AgentError> {
        let mut guard = entry.lock().await;
        let turn = match tokio::time::timeout(
            self.config.timeouts.prompt,
            guard.conn.prompt(prompt),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(AgentError::Timeout(format!(
                "architecture discovery (session {})",
                session.as_str()
            ))),
        }?;
        if turn.outcome == RunOutcome::Failed {
            return Err(AgentError::Prompt(
                "the Architect ended discovery without a valid answer".into(),
            ));
        }
        Ok(ArchitectReply {
            content: turn.text,
            acp_session_id: format!("acp-architecture:{}", session.as_str()),
            skill_name: guard.skill_name.clone(),
            skill_digest: guard.skill_digest.clone(),
            operating_mode: guard.operating_mode,
        })
    }
}

impl<C: Connector, D: ProjectDirs, R: RoutingSource> AgentChannel for AcpChannel<C, D, R> {
    async fn tell_manager(&self, project: &ProjectId, message: &str) -> PortResult<ManagerReply> {
        let entry = self.manager_for(project).await?;
        let mut guard = entry.lock().await;

        let preamble = self.preambles.for_role(
            &latoile_core::ids::RoleId::new("manager").expect("a fixed role id is non-empty"),
        );
        let prompt = if guard.greeted {
            message.to_string()
        } else {
            format!("{preamble}\n\n---\n\n{message}")
        };

        let turn =
            match tokio::time::timeout(self.config.timeouts.prompt, guard.conn.prompt(&prompt))
                .await
            {
                Ok(result) => result,
                Err(_) => Err(AgentError::Timeout(format!(
                    "prompt (project {})",
                    project.as_str()
                ))),
            };

        match turn {
            Ok(turn) => {
                guard.greeted = true;
                if turn.outcome == RunOutcome::Failed {
                    return Err(AgentError::Prompt(
                        "the manager ended the turn without answering".into(),
                    )
                    .into());
                }
                Ok(ManagerReply {
                    content: turn.text,
                    actions: None,
                })
            }
            Err(e) => {
                // A dead or wedged manager session is evicted: the next
                // message spawns a fresh one rather than piling onto a corpse.
                drop(guard);
                self.managers.lock().await.remove(project.as_str());
                Err(e.into())
            }
        }
    }

    async fn start_architecture(
        &self,
        project: &ProjectId,
        session: &ArchitectureSessionId,
        brief: &str,
    ) -> PortResult<ArchitectReply> {
        if self.architects.lock().await.contains_key(session.as_str()) {
            return Err(latoile_core::ports::PortError(
                "architecture session is already active".into(),
            ));
        }
        let dir = self
            .dirs
            .manager_dir(project)
            .await
            .ok_or_else(|| AgentError::NoWorkspace(format!("project {}", project.as_str())))
            .and_then(checked_dir)?;
        let command = self.routing.command_for("architect");
        let bundle = self.preambles.architect_bundle().map_err(|error| {
            AgentError::Prompt(format!("loading complete Architect skill bundle: {error}"))
        })?;
        let operating_mode = architecture_package::detect_operating_mode(&dir).await?;
        let mut conn = self
            .connector
            .connect(
                &command,
                &dir,
                PermissionContext {
                    role_id: "architect_discovery".into(),
                    run_id: None,
                    write_root: None,
                    broker: self.permissions.clone(),
                    timeout: self.config.timeouts.permission,
                },
            )
            .await?;
        conn.new_session(&dir).await?;
        let entry = Arc::new(Mutex::new(ArchitectEntry {
            conn,
            skill_name: bundle.name.clone(),
            skill_digest: bundle.digest.clone(),
            operating_mode,
        }));
        let prompt = format!(
            "{}\n\n---\n\nDISCOVERY-ONLY CONTRACT\nPinned operating mode: {}. Do not write files or execute commands yet. Ask one decision-rich question at a time. On this first turn, kind MUST be `question`; `ready_to_draft` is forbidden even if the brief appears complete. Return exactly one fenced `latoile-architecture` JSON object with schema_version 1, kind `question`, phase `domain_discovery`, `requirements` or `ux_discovery`, and message. Never invent an owner answer.\n\nINITIAL BRIEF\n{brief}",
            bundle.render(),
            operating_mode.as_str(),
        );
        let reply = self.architecture_prompt(&entry, session, &prompt).await?;
        self.architects
            .lock()
            .await
            .insert(session.as_str().to_string(), entry);
        Ok(reply)
    }

    async fn retry_architecture_question(
        &self,
        _project: &ProjectId,
        session: &ArchitectureSessionId,
    ) -> PortResult<ArchitectReply> {
        let entry = self
            .architects
            .lock()
            .await
            .get(session.as_str())
            .cloned()
            .ok_or_else(|| {
                latoile_core::ports::PortError(
                    "the live Architect session is unavailable; restart discovery".into(),
                )
            })?;
        let prompt = "DISCOVERY GUARD\nNo owner answer was supplied. Your first turn cannot be `ready_to_draft`. Identify one consequential unresolved owner decision and ask exactly one question. Return only the fenced `latoile-architecture` JSON contract with kind `question`; do not write files or execute commands.";
        match self.architecture_prompt(&entry, session, prompt).await {
            Ok(reply) => Ok(reply),
            Err(error) => {
                self.architects.lock().await.remove(session.as_str());
                Err(error.into())
            }
        }
    }

    async fn retry_architecture_contract(
        &self,
        _project: &ProjectId,
        session: &ArchitectureSessionId,
        current_phase: ArchitecturePhase,
    ) -> PortResult<ArchitectReply> {
        let entry = self
            .architects
            .lock()
            .await
            .get(session.as_str())
            .cloned()
            .ok_or_else(|| {
                latoile_core::ports::PortError(
                    "the live Architect session is unavailable; restart discovery".into(),
                )
            })?;
        let allowed_question_phases = match current_phase {
            ArchitecturePhase::DomainDiscovery => {
                "domain_discovery, requirements, or ux_discovery"
            }
            ArchitecturePhase::Requirements => "requirements or ux_discovery",
            ArchitecturePhase::UxDiscovery => "ux_discovery",
            ArchitecturePhase::ReadyToDraft => "none",
        };
        let prompt = format!(
            "CONTRACT REPAIR\nNo new owner answer was supplied. Your previous response did not satisfy the discovery protocol. Preserve the same underlying question or readiness decision; do not invent owner input. The persisted phase is `{}`. Return exactly one fenced `latoile-architecture` JSON object. For kind `question`, phase MUST be one of: {allowed_question_phases}. For kind `ready_to_draft`, phase MUST be `ready_to_draft`. Do not write files or execute commands.",
            current_phase.as_str(),
        );
        match self.architecture_prompt(&entry, session, &prompt).await {
            Ok(reply) => Ok(reply),
            Err(error) => {
                self.architects.lock().await.remove(session.as_str());
                Err(error.into())
            }
        }
    }

    async fn continue_architecture(
        &self,
        _project: &ProjectId,
        session: &ArchitectureSessionId,
        answer: &str,
    ) -> PortResult<ArchitectReply> {
        let entry = self
            .architects
            .lock()
            .await
            .get(session.as_str())
            .cloned()
            .ok_or_else(|| {
                latoile_core::ports::PortError(
                    "the live Architect session is unavailable; restart discovery".into(),
                )
            })?;
        let prompt = format!(
            "OWNER ANSWER\n{answer}\n\nContinue discovery. Ask exactly one next question or declare `ready_to_draft`. Return only the fenced `latoile-architecture` JSON contract."
        );
        match self.architecture_prompt(&entry, session, &prompt).await {
            Ok(reply) => Ok(reply),
            Err(error) => {
                self.architects.lock().await.remove(session.as_str());
                Err(error.into())
            }
        }
    }

    async fn cancel_architecture(&self, session: &ArchitectureSessionId) -> PortResult<()> {
        if let Some(entry) = self.architects.lock().await.remove(session.as_str()) {
            entry.lock().await.conn.cancel().await?;
        }
        Ok(())
    }

    async fn generate_architecture_package(
        &self,
        project: &ProjectId,
        session: &ArchitectureSessionId,
        request: &ArchitecturePackageRequest,
    ) -> PortResult<ArchitecturePackageReply> {
        // Discovery is complete. The generation turn starts from the durable
        // decisions in a separate, package-confined worktree.
        self.architects.lock().await.remove(session.as_str());
        let dir = self
            .dirs
            .manager_dir(project)
            .await
            .ok_or_else(|| AgentError::NoWorkspace(format!("project {}", project.as_str())))
            .and_then(checked_dir)?;
        let bundle = self.preambles.architect_bundle().map_err(|error| {
            AgentError::Prompt(format!("loading complete Architect skill bundle: {error}"))
        })?;
        let command = self.routing.command_for("architect");
        architecture_package::generate(
            &self.connector,
            &command,
            &dir,
            session,
            request,
            &bundle,
            PermissionContext {
                role_id: "architect_package".into(),
                run_id: None,
                write_root: None,
                broker: self.permissions.clone(),
                timeout: self.config.timeouts.permission,
            },
            self.config.timeouts,
        )
        .await
        .map_err(Into::into)
    }

    async fn verify_architecture_package(
        &self,
        project: &ProjectId,
        spec: &SpecVersion,
    ) -> PortResult<ArchitecturePackageValidation> {
        let dir = self
            .dirs
            .manager_dir(project)
            .await
            .ok_or_else(|| AgentError::NoWorkspace(format!("project {}", project.as_str())))
            .and_then(checked_dir)?;
        Ok(architecture_package::verify_existing(&dir, spec).await)
    }

    async fn read_architecture_artifact(
        &self,
        project: &ProjectId,
        spec: &SpecVersion,
        relative_path: &str,
    ) -> PortResult<String> {
        let dir = self
            .dirs
            .manager_dir(project)
            .await
            .ok_or_else(|| AgentError::NoWorkspace(format!("project {}", project.as_str())))
            .and_then(checked_dir)?;
        architecture_package::read_artifact(&dir, spec, relative_path)
            .await
            .map_err(Into::into)
    }

    async fn start_run(&self, project: &ProjectId, run: &Run, prompt: &str) -> PortResult<String> {
        let dir = self
            .dirs
            .run_dir(project, run)
            .await
            .ok_or_else(|| AgentError::NoWorkspace(format!("run {}", run.id.as_str())))
            .and_then(checked_dir)?;
        let command = self.routing.command_for(run.role_id.as_str());
        let mut conn = self
            .connector
            .connect(
                &command,
                &dir,
                PermissionContext {
                    role_id: run.role_id.as_str().to_string(),
                    run_id: Some(run.id.clone()),
                    write_root: None,
                    broker: self.permissions.clone(),
                    timeout: self.config.timeouts.permission,
                },
            )
            .await?;
        conn.new_session(&dir).await?;

        let preamble = self.preambles.for_role(&run.role_id);
        let full_prompt = format!("{preamble}\n\n---\n\n{prompt}");
        let timeout = self.config.timeouts.prompt;
        let base_sha = git_output(&dir, &["rev-parse", "HEAD"]).await;
        let evidence_dir = dir.clone();

        let state = Arc::new(StdMutex::new(RunState::Running));
        let task_state = state.clone();
        let permission_broker = self.permissions.clone();
        let permission_run = run.id.clone();
        let task = tokio::spawn(async move {
            let recorded = match tokio::time::timeout(timeout, conn.prompt(&full_prompt)).await {
                Ok(Ok(turn)) => {
                    let activity = turn
                        .updates
                        .iter()
                        .filter_map(activity_summary)
                        .take(200)
                        .collect();
                    let git = collect_git_evidence(&evidence_dir, base_sha).await;
                    RunState::Done(RunReport {
                        outcome: turn.outcome,
                        summary: truncate(turn.text.trim().to_string(), 64 * 1024),
                        activity,
                        base_sha: git.base_sha,
                        head_sha: git.head_sha,
                        commits: git.commits,
                        changed_files: git.changed_files,
                        diff_stat: git.diff_stat,
                    })
                }
                Ok(Err(e)) => RunState::Failed(e.to_string()),
                Err(_) => RunState::Failed("timed out".into()),
            };
            permission_broker.finish_run(&permission_run);
            *task_state.lock().expect("run state poisoned") = recorded;
            // `conn` drops here; the agent process dies with it.
        });

        let session_id = format!("acp:{}", run.id.as_str());
        self.runs.lock().await.insert(
            run.id.as_str().to_string(),
            RunEntry {
                abort: task.abort_handle(),
                state,
            },
        );
        Ok(session_id)
    }

    async fn cancel_run(&self, run: &RunId) -> PortResult<()> {
        self.permissions.cancel_run(run);
        // Unknown run: already gone is the wanted state — fine.
        if let Some(entry) = self.runs.lock().await.remove(run.as_str()) {
            entry.abort.abort(); // the connection — and the process — die
            *entry.state.lock().expect("run state poisoned") =
                RunState::Done(RunReport::terminal(RunOutcome::Cancelled, ""));
        }
        Ok(())
    }

    async fn resolve_permission(
        &self,
        run: &RunId,
        request_id: &str,
        granted: bool,
    ) -> PortResult<()> {
        self.permissions
            .resolve(run, request_id, granted)
            .map_err(latoile_core::ports::PortError)
    }
}

fn activity_summary(update: &AgentUpdate) -> Option<String> {
    match update {
        // ACP titles and permission summaries can contain raw command
        // arguments, so persist only the lifecycle signal.
        AgentUpdate::ToolCallStarted { .. } => Some("tool call started".into()),
        AgentUpdate::ToolCallFinished { .. } => Some("tool call finished".into()),
        AgentUpdate::PermissionRequested { .. } => Some("permission requested".into()),
        AgentUpdate::PlanUpdated => Some("plan updated".into()),
        AgentUpdate::TextChunk(_) | AgentUpdate::ThoughtChunk(_) | AgentUpdate::Ignored(_) => None,
    }
}

#[derive(Default)]
struct GitEvidence {
    base_sha: Option<String>,
    head_sha: Option<String>,
    commits: Vec<CommitEvidence>,
    changed_files: Vec<ChangedFileEvidence>,
    diff_stat: String,
}

async fn collect_git_evidence(dir: &std::path::Path, base_sha: Option<String>) -> GitEvidence {
    let head_sha = git_output(dir, &["rev-parse", "HEAD"]).await;
    let range = match (&base_sha, &head_sha) {
        (Some(base), Some(head)) if base != head => Some(format!("{base}..{head}")),
        _ => None,
    };

    let commits = match range.as_deref() {
        Some(range) => git_output(dir, &["log", "--format=%H%x09%s", range])
            .await
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(sha, subject)| CommitEvidence {
                sha: sha.to_string(),
                subject: truncate(subject.to_string(), 512),
            })
            .take(100)
            .collect(),
        None => Vec::new(),
    };

    let mut changed_files = Vec::new();
    if let Some(range) = range.as_deref() {
        parse_name_status(
            &git_output(dir, &["diff", "--name-status", range])
                .await
                .unwrap_or_default(),
            &mut changed_files,
        );
    }
    parse_porcelain(
        &git_output(dir, &["status", "--porcelain=v1"])
            .await
            .unwrap_or_default(),
        &mut changed_files,
    );
    changed_files.sort_by(|a, b| a.path.cmp(&b.path));
    changed_files.dedup_by(|a, b| a.path == b.path);
    changed_files.truncate(500);

    let diff_stat = match range.as_deref() {
        Some(range) => git_output(dir, &["diff", "--stat", range])
            .await
            .unwrap_or_default(),
        None => String::new(),
    };

    GitEvidence {
        base_sha,
        head_sha,
        commits,
        changed_files,
        diff_stat: truncate(diff_stat, 32 * 1024),
    }
}

fn parse_name_status(raw: &str, out: &mut Vec<ChangedFileEvidence>) {
    for line in raw.lines() {
        if let Some((status, path)) = line.split_once('\t') {
            out.push(ChangedFileEvidence {
                status: truncate(status.to_string(), 16),
                path: truncate(path.to_string(), 2048),
            });
        }
    }
}

fn parse_porcelain(raw: &str, out: &mut Vec<ChangedFileEvidence>) {
    for line in raw.lines() {
        if line.len() >= 4 {
            out.push(ChangedFileEvidence {
                status: truncate(line[..2].trim().to_string(), 16),
                path: truncate(line[3..].to_string(), 2048),
            });
        }
    }
}

async fn git_output(dir: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn truncate(mut value: String, limit: usize) -> String {
    if value.len() > limit {
        let mut boundary = limit;
        while !value.is_char_boundary(boundary) {
            boundary -= 1;
        }
        value.truncate(boundary);
        value.push_str("\n… truncated");
    }
    value
}

#[cfg(test)]
mod tests;
