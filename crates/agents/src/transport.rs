//! The ACP transport: one spawned agent process, one JSON-RPC connection, one
//! session. This is the only module that talks to the SDK — `channel.rs`
//! builds the `AgentChannel` port on the [`Connection`]/[`Connector`]
//! abstraction, and tests drive scripted fakes instead of processes.
//!
//! Process supervision is the SDK's `AcpAgent`: spawn in its own process
//! group, kill the group when the connection drops. Dropping a
//! [`ProcessConnection`] aborts the actor task, which drops the connection,
//! which kills the agent — no orphans (contract §3).

use crate::config::AgentCommand;
use crate::error::AgentError;
use crate::updates::{classify, outcome_of, AgentUpdate, RunOutcome};
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, InitializeRequest, NewSessionRequest, PermissionOptionKind,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, StopReason, TextContent,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Client, ConnectionTo, Responder};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// What one prompt turn produced.
#[derive(Debug)]
pub struct TurnResult {
    pub outcome: RunOutcome,
    /// The concatenated text chunks — the agent's reply.
    pub text: String,
    /// Everything the agent sent, in order.
    pub updates: Vec<AgentUpdate>,
}

/// One ACP session on one connection. Signatures are desugared (`impl
/// Future + Send`) rather than `async fn` so the returned futures are
/// provably `Send` — the channel spawns run turns onto background tasks.
pub trait Connection: Send {
    fn new_session<'a>(
        &'a mut self,
        cwd: &'a Path,
    ) -> impl std::future::Future<Output = Result<(), AgentError>> + Send + 'a;
    fn prompt<'a>(
        &'a mut self,
        text: &'a str,
    ) -> impl std::future::Future<Output = Result<TurnResult, AgentError>> + Send + 'a;
    /// `session/cancel` — best effort: the prompt resolves with
    /// `StopReason::Cancelled` when the agent honours it.
    fn cancel(&mut self) -> impl std::future::Future<Output = Result<(), AgentError>> + Send + '_;
}

/// How connections are made. The real one spawns processes; tests script.
/// `'static` connections: a run's turn outlives the call that started it.
pub trait Connector: Send + Sync {
    type Conn: Connection + 'static;
    fn connect<'a>(
        &'a self,
        command: &'a AgentCommand,
        workspace: &'a Path,
    ) -> impl std::future::Future<Output = Result<Self::Conn, AgentError>> + Send + 'a;
}

// --- The real thing ---------------------------------------------------------

/// Commands the connection handle sends to the actor task that owns the
/// SDK connection (the SDK hands us the connection inside a closure, so the
/// connection has to live on its own task).
enum Cmd {
    NewSession {
        cwd: PathBuf,
        resp: oneshot::Sender<Result<String, String>>,
    },
    Prompt {
        text: String,
        resp: oneshot::Sender<Result<StopReason, String>>,
    },
    Cancel,
}

pub struct ProcessConnector {
    pub handshake: Duration,
}

// Desugared (not `async fn`) on purpose: the trait needs the explicit
// `+ Send` on the returned future, which `async fn` cannot express.
#[allow(clippy::manual_async_fn)]
impl Connector for ProcessConnector {
    type Conn = ProcessConnection;

    fn connect<'a>(
        &'a self,
        command: &'a AgentCommand,
        workspace: &'a Path,
    ) -> impl std::future::Future<Output = Result<ProcessConnection, AgentError>> + Send + 'a {
        async move { ProcessConnection::spawn(command, workspace, self.handshake).await }
    }
}

/// One live agent process and its ACP connection. Dropping it kills the
/// process (via the aborted actor and the SDK's process-group guard).
pub struct ProcessConnection {
    cmd: mpsc::Sender<Cmd>,
    updates: mpsc::UnboundedReceiver<AgentUpdate>,
    actor: JoinHandle<()>,
    session: Option<String>,
    handshake: Duration,
    workspace: PathBuf,
}

impl Drop for ProcessConnection {
    fn drop(&mut self) {
        // Aborting the actor drops the SDK connection; the SDK's child guard
        // then kills the process group.
        self.actor.abort();
    }
}

impl ProcessConnection {
    async fn spawn(
        command: &AgentCommand,
        workspace: &Path,
        handshake: Duration,
    ) -> Result<Self, AgentError> {
        let mut config = AcpAgentConfig::new(&command.program).args(command.args.clone());
        for (name, value) in &command.env {
            config = config.env(name, value);
        }
        let agent = AcpAgent::new(config);

        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>(8);
        let (updates_tx, updates_rx) = mpsc::unbounded_channel::<AgentUpdate>();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

        let notif_tx = updates_tx.clone();
        let perm_tx = updates_tx;
        let perm_workspace = workspace.to_path_buf();
        let actor_workspace = workspace.to_path_buf();

        let actor = tokio::spawn(async move {
            let run = Client
                .builder()
                .on_receive_notification(
                    move |notification: SessionNotification, _cx| {
                        let tx = notif_tx.clone();
                        async move {
                            let _ = tx.send(classify(&notification.update));
                            Ok(())
                        }
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    move |request: RequestPermissionRequest, responder, _conn| {
                        let tx = perm_tx.clone();
                        let workspace = perm_workspace.clone();
                        async move { answer_permission(request, responder, tx, workspace) }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent, move |conn| async move {
                    actor(conn, cmd_rx, ready_tx, handshake, actor_workspace).await
                })
                .await;
            // The connection ending is not necessarily an error worth
            // surfacing here — waiters learn about it as `AgentGone`.
            let _ = run;
        });

        match ready_rx.await {
            Ok(Ok(())) => Ok(Self {
                cmd: cmd_tx,
                updates: updates_rx,
                actor,
                session: None,
                handshake,
                workspace: workspace.to_path_buf(),
            }),
            Ok(Err(e)) => Err(AgentError::Handshake(e)),
            Err(_) => Err(AgentError::AgentGone),
        }
    }
}

/// The permission answer: the policy decides, the closest matching option is
/// selected, and the request is surfaced as an update.
fn answer_permission(
    request: RequestPermissionRequest,
    responder: Responder<RequestPermissionResponse>,
    updates: mpsc::UnboundedSender<AgentUpdate>,
    workspace: PathBuf,
) -> Result<(), agent_client_protocol::Error> {
    let title = request.tool_call.fields.title.as_deref();
    let summary = title.unwrap_or("tool call").to_string();
    let _ = updates.send(AgentUpdate::PermissionRequested {
        summary: summary.clone(),
    });

    let decision = crate::policy::decide(
        title,
        request.tool_call.fields.raw_input.as_ref(),
        &workspace,
    );
    let wanted = match decision {
        crate::policy::Decision::AllowOnce => PermissionOptionKind::AllowOnce,
        crate::policy::Decision::Reject => PermissionOptionKind::RejectOnce,
    };
    let option = request
        .options
        .iter()
        .find(|o| o.kind == wanted)
        .or(request.options.first());

    responder.respond(RequestPermissionResponse::new(match option {
        Some(o) => {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(o.option_id.clone()))
        }
        None => RequestPermissionOutcome::Cancelled,
    }))
}

/// The connection's own task: handshake, then relay commands until the handle
/// goes away.
async fn actor(
    conn: ConnectionTo<agent_client_protocol::Agent>,
    mut rx: mpsc::Receiver<Cmd>,
    ready: oneshot::Sender<Result<(), String>>,
    handshake: Duration,
    workspace: PathBuf,
) -> Result<(), agent_client_protocol::Error> {
    let init = conn
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task();
    let _ = ready.send(match tokio::time::timeout(handshake, init).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!(
            "initialize timed out (cwd: {})",
            workspace.display()
        )),
    });

    let mut session: Option<agent_client_protocol::schema::v1::SessionId> = None;
    while let Some(cmd) = rx.recv().await {
        match cmd {
            Cmd::NewSession { cwd, resp } => {
                let result = conn
                    .send_request(NewSessionRequest::new(cwd))
                    .block_task()
                    .await;
                match result {
                    Ok(r) => {
                        session = Some(r.session_id.clone());
                        let _ = resp.send(Ok(r.session_id.to_string()));
                    }
                    Err(e) => {
                        let _ = resp.send(Err(e.to_string()));
                    }
                }
            }
            Cmd::Prompt { text, resp } => {
                let Some(session) = &session else {
                    let _ = resp.send(Err("prompt before session/new".into()));
                    continue;
                };
                let result = conn
                    .send_request(agent_client_protocol::schema::v1::PromptRequest::new(
                        session.clone(),
                        vec![ContentBlock::Text(TextContent::new(text))],
                    ))
                    .block_task()
                    .await;
                let _ = resp.send(result.map(|r| r.stop_reason).map_err(|e| e.to_string()));
            }
            Cmd::Cancel => {
                if let Some(session) = &session {
                    conn.send_notification(CancelNotification::new(session.clone()))?;
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::manual_async_fn)] // same reason as `Connector` above
impl Connection for ProcessConnection {
    fn new_session<'a>(
        &'a mut self,
        cwd: &'a Path,
    ) -> impl std::future::Future<Output = Result<(), AgentError>> + Send + 'a {
        async move { self.open_session(cwd).await }
    }

    fn prompt<'a>(
        &'a mut self,
        text: &'a str,
    ) -> impl std::future::Future<Output = Result<TurnResult, AgentError>> + Send + 'a {
        async move { self.run_prompt(text).await }
    }

    fn cancel(&mut self) -> impl std::future::Future<Output = Result<(), AgentError>> + Send + '_ {
        async move { self.send_cancel().await }
    }
}

impl ProcessConnection {
    async fn open_session(&mut self, cwd: &Path) -> Result<(), AgentError> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::NewSession {
                cwd: cwd.to_path_buf(),
                resp: tx,
            })
            .await
            .map_err(|_| AgentError::AgentGone)?;
        let id = tokio::time::timeout(self.handshake, rx)
            .await
            .map_err(|_| {
                AgentError::Timeout(format!("session/new (cwd: {})", self.workspace.display()))
            })?
            .map_err(|_| AgentError::AgentGone)?
            .map_err(AgentError::Session)?;
        self.session = Some(id);
        Ok(())
    }

    async fn run_prompt(&mut self, text: &str) -> Result<TurnResult, AgentError> {
        if self.session.is_none() {
            return Err(AgentError::Session("prompt before session/new".into()));
        }
        // Discard anything left over from a previous turn.
        while self.updates.try_recv().is_ok() {}

        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::Prompt {
                text: text.to_string(),
                resp: tx,
            })
            .await
            .map_err(|_| AgentError::AgentGone)?;

        let mut reply = String::new();
        let mut updates = Vec::new();
        let mut rx = std::pin::pin!(rx);
        let stop = loop {
            tokio::select! {
                resp = &mut rx => break resp.map_err(|_| AgentError::AgentGone)?,
                Some(update) = self.updates.recv() => {
                    if let AgentUpdate::TextChunk(chunk) = &update {
                        reply.push_str(chunk);
                    }
                    updates.push(update);
                }
            }
        };
        let stop = stop.map_err(AgentError::Prompt)?;
        Ok(TurnResult {
            outcome: outcome_of(&stop),
            text: reply,
            updates,
        })
    }

    async fn send_cancel(&mut self) -> Result<(), AgentError> {
        self.cmd
            .send(Cmd::Cancel)
            .await
            .map_err(|_| AgentError::AgentGone)
    }
}
