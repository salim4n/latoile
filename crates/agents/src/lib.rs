//! The agents adapter — the only crate allowed to spawn agent processes
//! (architecture contract §3). It implements `latoile_core::ports::AgentChannel`
//! by talking ACP (Agent Client Protocol) to locally installed agent CLIs:
//! `claude-agent-acp` wrapping Claude Code, `codex-acp` wrapping Codex. Both
//! carry their own auth from the user's machine; this crate never touches an
//! agent API key.
//!
//! Layout:
//!
//! - [`config`] — which binary for which role, and time budgets. Exceeding a
//!   budget kills the process; a hung agent is never left behind.
//! - [`preamble`] — role → skill preamble, read from an injected directory.
//! - [`policy`] — the permission answer given to agents (pure, fail-closed
//!   on `.env`, `docker`, and absolute paths outside the workspace).
//! - [`updates`] — ACP session updates → the channel's vocabulary, and how
//!   turn outcomes map onto the domain's `EventKind`s (pure).
//! - `transport` — the SDK actor. The only module that knows ACP wire types
//!   at runtime; everything above it is testable without a process.
//! - [`channel`] — the `AgentChannel` implementation itself.

mod auth;
mod channel;
mod config;
mod error;
mod policy;
mod preamble;
mod transport;
mod updates;

pub use auth::{
    AgentAuthManager, AuthError, AuthProvider, AuthSessionView, AuthStatus, ProviderCommands,
    ProviderStatus, DEFAULT_TTL,
};
pub use channel::{AcpChannel, ProjectDirs, RootDirs, RoutingSource, RunState, SharedRouting};
pub use config::{AgentCommand, AgentTimeouts, ChannelConfig};
pub use error::AgentError;
pub use updates::{AgentUpdate, RunOutcome};

pub use updates::{classify, outcome_event, outcome_of, update_event};

/// The production connector: spawns real agent processes. Everything else in
/// the crate is testable without one.
pub use transport::{Connection, Connector, ProcessConnector, TurnResult};
