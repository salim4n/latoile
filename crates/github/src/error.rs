//! What the GitHub client reports. The token is never part of an error —
//! the secret's *name* is not a secret. Mapped into the opaque `PortError`
//! at the port boundary (contract §5).

use latoile_core::ports::PortError;

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    /// No token stored under the configured secret name.
    #[error("no GitHub token in the vault under {0:?} — add one in Settings")]
    TokenMissing(String),
    /// 401 or 403: the token is wrong, expired, or lacks the scope.
    #[error("GitHub refused the token (check its scope and expiry)")]
    Auth,
    /// 404: the repository (or the token's view of it) does not exist.
    #[error("not found on GitHub: {0}")]
    NotFound(String),
    /// 422: GitHub's own validation message, surfaced — it says exactly what
    /// was rejected (a PR that already exists, a missing branch…).
    #[error("GitHub rejected the request: {0}")]
    Validation(String),
    /// DNS, TCP, TLS — the network itself failed.
    #[error("talking to GitHub failed: {0}")]
    Http(#[from] reqwest::Error),
    /// A 200 that wasn't the JSON the API promised.
    #[error("an unexpected GitHub response: {0}")]
    Decode(String),
    /// Local checkout or Git command failure. Messages are sanitized before
    /// reaching this variant, so credentials can never enter logs.
    #[error("provisioning the repository failed: {0}")]
    Workspace(String),
}

impl From<GitHubError> for PortError {
    fn from(e: GitHubError) -> Self {
        PortError(e.to_string())
    }
}
