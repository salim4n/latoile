//! GitHub integration. Lists the owner's repositories for the project picker,
//! manages branches, and opens pull requests. Tokens come from `latoile-vault`,
//! never from configuration files or the environment at call sites.
//!
//! Implements ports defined in `latoile-core`.

mod client;
mod error;
mod workspace;

pub use client::{GitHub, GitHubConfig, DEFAULT_TOKEN_NAME};
pub use error::GitHubError;
