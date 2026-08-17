//! What the agent channel reports. Values and transcripts are never part of
//! an error — commands and phases are. Mapped into the opaque `PortError` at
//! the port boundary (contract §5).

use latoile_core::ports::PortError;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// The binary could not be launched at all (not installed, not on PATH).
    #[error("spawning the agent failed: {0}")]
    Spawn(String),
    /// `initialize` did not complete — the process is not an ACP agent, or
    /// not a healthy one.
    #[error("the ACP handshake failed: {0}")]
    Handshake(String),
    /// `session/new` failed.
    #[error("creating the ACP session failed: {0}")]
    Session(String),
    /// `session/prompt` failed or the agent ended the turn for a reason that
    /// is a failure to us (refusal, token limit).
    #[error("the prompt failed: {0}")]
    Prompt(String),
    /// A phase exceeded its configured budget. The process is killed rather
    /// than left running (contract §3: no orphans). The message names the
    /// phase AND the working directory — a bare "timed out" is undebuggable.
    #[error("the agent timed out during {0}")]
    Timeout(String),
    /// The process died or closed the transport while we were waiting on it.
    #[error("the agent process is gone")]
    AgentGone,
    /// The project/run has no resolvable working directory.
    #[error("no working directory known for {0}")]
    NoWorkspace(String),
}

impl From<AgentError> for PortError {
    fn from(e: AgentError) -> Self {
        PortError(e.to_string())
    }
}
