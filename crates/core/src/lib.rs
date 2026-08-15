//! The domain. Zero I/O, zero async, zero external dependencies — this crate
//! must compile for any target and never know how it is persisted or served.
//!
//! Owns: entities (`Project`, `SpecVersion`, `Task`, `Run`, `Approval`,
//! `Preview`, `Conversation`, `Message`), the state machines with their
//! exhaustive and refused transitions, domain events (`Event`), and the ports
//! (traits) that `latoile-app` orchestrates and the adapters implement.
//!
//! Invariants enforced here (see docs/architecture-spec.md §3.2):
//! one active run per task, one approved spec per project, `done` requires a
//! granted review approval, the Manager never executes.

pub mod error;
pub mod event;
pub mod ids;

pub use error::{DomainError, TransitionError};
pub use event::{EventKind, NewEvent};
pub use ids::{
    ApprovalId, ConversationId, MessageId, PreviewId, ProjectId, RoleId, RunId, SpecVersionId,
    TaskId,
};
