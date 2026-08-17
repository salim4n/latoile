//! One use case per file. Every use case follows the same six steps:
//! validate input → fetch entities → call domain methods → save → publish
//! events → return a DTO.
//!
//! Use cases are generic over the ports: `async fn` in traits (Rust-native
//! RPITIT) makes the traits non-dyn-compatible, so constructor injection uses
//! type parameters instead of `Arc<dyn …>`.

mod approve_spec;
mod create_project;
mod dispatch_task;
mod ensure_preview;
mod grant_approval;
mod manager_turn;
mod reject_approval;
mod routing;
mod send_message;
mod stop_preview;

pub use approve_spec::ApproveSpec;
pub use create_project::{CreateProject, CreateProjectInput};
pub use dispatch_task::{DispatchTask, DispatchTaskInput, DispatchedTask};
pub use ensure_preview::{EnsurePreview, EnsuredPreview};
pub use grant_approval::{GrantApproval, GrantedApproval};
pub use manager_turn::{ManagerOutcome, ManagerTurn};
pub use reject_approval::RejectApproval;
pub use routing::{RoleRouting, Routing, PROVIDERS, ROLES};
pub use send_message::{PostedMessage, SendMessage, SendMessageInput};
pub use stop_preview::StopPreview;

use latoile_core::error::DomainError;
use latoile_core::ports::PortError;

/// What a use case reports. The server maps this to `{code, message}` and
/// keeps internal detail out of the response (contract §5).
#[derive(Debug, thiserror::Error)]
pub enum UseCaseError {
    /// The domain refused: invalid transition or violated invariant.
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// An adapter or the store failed.
    #[error(transparent)]
    Port(#[from] PortError),
    /// The input points at something that does not exist.
    #[error("not found: {0}")]
    NotFound(&'static str),
}
