//! The HTTP/SSE edge — the only crate that knows axum (contract §1).
//! Handlers extract, validate, delegate to `latoile-app`; all state machines,
//! all SQL, all processes live elsewhere.
//!
//! - [`state::build`] wires the concrete adapters for the CLI.
//! - [`routes::router`] assembles the API contract (spec §5.3) behind the
//!   bearer token (D9); `/api/health` is the only open route.
//! - Errors are `{code, message}` everywhere; internals go to `tracing`.

mod auth;
mod error;
mod routes;
mod state;

pub use error::ApiError;
pub use routes::router;
pub use state::{build, AgentSlot, AppState, BuildError, GitHubSlot, ServerConfig, TOKEN_ENV};

#[cfg(test)]
mod tests;
