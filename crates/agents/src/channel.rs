//! The `AgentChannel` port. Two lifecycles, per the port's contract:
//!
//! - **Manager**: one persistent ACP session per project, spawned on first
//!   message, resumed after. The role's skill preamble heads the first
//!   prompt only.
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
//! recorded here (`run_state`) for the app layer to pick up, because the
//! port has no completion callback. That is the one seam a future wiring
//! step should close.

use crate::config::ChannelConfig;
use crate::error::AgentError;
use crate::preamble::Preambles;
use crate::transport::{Connection, Connector};
use crate::updates::RunOutcome;
use latoile_core::ids::{ProjectId, RunId};
use latoile_core::ports::{AgentChannel, ManagerReply, PortResult};
use latoile_core::Run;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

/// Where sessions work. Sync and simple: resolving a directory must not need
/// the database — the CLI assembles whatever lookup it wants behind this.
pub trait ProjectDirs: Send + Sync {
    fn manager_dir(&self, project: &ProjectId) -> Option<PathBuf>;
    fn run_dir(&self, run: &Run) -> Option<PathBuf>;
}

/// Every session in one root directory. The workable V1 default: ACP agents
/// scope file tools to the session `cwd`, and the prompt text carries the
/// exact checkout path.
pub struct RootDirs(pub PathBuf);

impl ProjectDirs for RootDirs {
    fn manager_dir(&self, _project: &ProjectId) -> Option<PathBuf> {
        Some(self.0.clone())
    }
    fn run_dir(&self, _run: &Run) -> Option<PathBuf> {
        Some(self.0.clone())
    }
}

/// Where a background run stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    Running,
    Done(RunOutcome),
    /// Transport or timeout failure; the message is for logs, not clients.
    Failed(String),
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

pub struct AcpChannel<C: Connector, D: ProjectDirs> {
    config: ChannelConfig,
    connector: C,
    dirs: D,
    preambles: Preambles,
    managers: Mutex<HashMap<String, ManagerSlot<C::Conn>>>,
    runs: Mutex<HashMap<String, RunEntry>>,
}

impl<C: Connector, D: ProjectDirs> AcpChannel<C, D> {
    pub fn new(config: ChannelConfig, connector: C, dirs: D) -> Self {
        let preambles = Preambles::new(config.skills_dir.clone());
        Self {
            config,
            connector,
            dirs,
            preambles,
            managers: Mutex::new(HashMap::new()),
            runs: Mutex::new(HashMap::new()),
        }
    }

    /// Where a run stands, for the app layer polling from its own loop.
    /// `None` = unknown run.
    pub async fn run_state(&self, run: &RunId) -> Option<RunState> {
        let runs = self.runs.lock().await;
        runs.get(run.as_str())
            .map(|e| e.state.lock().expect("run state poisoned").clone())
    }

    async fn manager_for(
        &self,
        project: &ProjectId,
    ) -> Result<ManagerSlot<C::Conn>, AgentError> {
        let mut managers = self.managers.lock().await;
        if let Some(entry) = managers.get(project.as_str()) {
            return Ok(entry.clone());
        }
        let dir = self
            .dirs
            .manager_dir(project)
            .ok_or_else(|| AgentError::NoWorkspace(format!("project {}", project.as_str())))?;
        let mut conn = self
            .connector
            .connect(self.config.command_for("manager"), &dir)
            .await?;
        conn.new_session(&dir).await?;
        let entry = Arc::new(Mutex::new(ManagerEntry {
            conn,
            greeted: false,
        }));
        managers.insert(project.as_str().to_string(), entry.clone());
        Ok(entry)
    }
}

impl<C: Connector, D: ProjectDirs> AgentChannel for AcpChannel<C, D> {
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

        let turn = match tokio::time::timeout(self.config.timeouts.prompt, guard.conn.prompt(&prompt))
            .await
        {
            Ok(result) => result,
            Err(_) => Err(AgentError::Timeout("prompt")),
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

    async fn start_run(&self, run: &Run, prompt: &str) -> PortResult<String> {
        let dir = self
            .dirs
            .run_dir(run)
            .ok_or_else(|| AgentError::NoWorkspace(format!("run {}", run.id.as_str())))?;
        let command = self.config.command_for(run.role_id.as_str());
        let mut conn = self.connector.connect(command, &dir).await?;
        conn.new_session(&dir).await?;

        let preamble = self.preambles.for_role(&run.role_id);
        let full_prompt = format!("{preamble}\n\n---\n\n{prompt}");
        let timeout = self.config.timeouts.prompt;

        let state = Arc::new(StdMutex::new(RunState::Running));
        let task_state = state.clone();
        let task = tokio::spawn(async move {
            let recorded = match tokio::time::timeout(timeout, conn.prompt(&full_prompt)).await {
                Ok(Ok(turn)) => RunState::Done(turn.outcome),
                Ok(Err(e)) => RunState::Failed(e.to_string()),
                Err(_) => RunState::Failed("timed out".into()),
            };
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
        // Unknown run: already gone is the wanted state — fine.
        if let Some(entry) = self.runs.lock().await.remove(run.as_str()) {
            entry.abort.abort(); // the connection — and the process — die
            *entry.state.lock().expect("run state poisoned") = RunState::Done(RunOutcome::Cancelled);
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests;
