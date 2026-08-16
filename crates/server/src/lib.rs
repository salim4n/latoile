//! The HTTP/SSE edge — the only crate that knows axum (contract §1).
//! Handlers extract, validate, delegate to `latoile-app`; all state machines,
//! all SQL, all processes live elsewhere.
//!
//! - [`state::build`] wires the concrete adapters for the CLI.
//! - [`routes::router`] assembles the API contract (spec §5.3) behind the
//!   bearer token (D9); `/api/health` is the only open route.
//! - Errors are `{code, message}` everywhere; internals go to `tracing`.

mod assets;
mod auth;
mod error;
mod routes;
mod state;

pub use error::ApiError;
pub use routes::router;
pub use state::{build, AgentSlot, AppState, BuildError, GitHubSlot, ServerConfig, TOKEN_ENV};

#[cfg(test)]
mod tests;

/// Run the assembled router until the shutdown future resolves. Lives here
/// so the CLI never names axum (guardian #2: HTTP is confined to this crate).
pub async fn serve(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
}
