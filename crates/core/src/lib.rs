//! The domain. Zero I/O and zero external dependencies — native async port
//! signatures declare adapter contracts without owning a runtime. This crate
//! never knows how it is persisted or served.
//!
//! Owns: entities (`Project`, `SpecVersion`, `Task`, `Run`, `Approval`,
//! `Preview`, `Conversation`, `Message`), the state machines with their
//! exhaustive and refused transitions, domain events (`Event`), and the ports
//! (traits) that `latoile-app` orchestrates and the adapters implement.
//!
//! Invariants enforced here (see docs/architecture-spec.md §3.2):
//! one active run per task, one approved spec per project, `done` requires a
//! granted review approval, the Manager never executes.

pub mod approval;
pub mod architecture;
pub mod conversation;
pub mod delivery;
pub mod error;
pub mod event;
pub mod ids;
#[allow(async_fn_in_trait)] // see ports.rs module docs for the rationale
pub mod ports;
pub mod preview;
pub mod project;
pub mod run;
pub mod spec;
pub mod task;

pub use approval::{Approval, ApprovalKind, ApprovalStatus};
pub use architecture::{
    ArchitectureOperatingMode, ArchitecturePackageEvidence, ArchitecturePackageStatus,
    ArchitecturePhase, ArchitectureQuestion, ArchitectureQuestionStatus, ArchitectureSession,
    ArchitectureStatus, ARCHITECT_SKILL_NAME,
};
pub use conversation::{Author, Conversation, Message};
pub use delivery::{Delivery, DeliveryStatus};
pub use error::{DomainError, TransitionError};
pub use event::{EventKind, NewEvent};
pub use ids::{
    ApprovalId, ArchitectureQuestionId, ArchitectureSessionId, ConversationId, MessageId,
    PreviewId, ProjectId, RoleId, RunId, SpecVersionId, TaskId,
};
pub use preview::{Preview, PreviewStatus};
pub use project::{Project, ProjectStatus};
pub use run::{Run, RunStatus, TriggeredBy};
pub use spec::{SpecProvenance, SpecStatus, SpecVersion};
pub use task::{Task, TaskStatus};
