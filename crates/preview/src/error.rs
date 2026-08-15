//! What preview supervision reports. Mapped into the opaque `PortError` at
//! the port boundary; log lines quoted in errors are the dev server's own
//! output, never secrets (contract §5).

use latoile_core::ports::PortError;

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    /// The dev command could not be launched at all.
    #[error("spawning the dev server failed: {0}")]
    Spawn(String),
    /// Every port in the allocatable range is taken.
    #[error("no free port in the preview range")]
    NoFreePort,
    /// The process started but nothing was listening when the budget ran
    /// out. Carries the last log lines — that is where the why lives.
    #[error("the dev server was not ready in time; last output: {0}")]
    NotReady(String),
    /// The process exited before ever serving.
    #[error("the dev server exited before becoming ready; last output: {0}")]
    Exited(String),
}

impl From<PreviewError> for PortError {
    fn from(e: PreviewError) -> Self {
        PortError(e.to_string())
    }
}
