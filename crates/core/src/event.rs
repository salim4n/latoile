//! Domain events — the things that happen and the rest of the system cares
//! about. Persisted in the append-only `EVENT` log by the application layer;
//! `seq` (assigned by the store) is the SSE cursor.

use crate::ids::ProjectId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    SpecVersionCreated,
    SpecApproved,
    TaskReady,
    RunStarted,
    RunBlocked,
    RunFinished,
    ApprovalRequested,
    ApprovalGranted,
    ApprovalRejected,
    PreviewReady,
    PreviewStale,
    PreviewError,
    MessagePosted,
}

impl EventKind {
    /// Stable wire name, used in the event log and on the SSE stream.
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::SpecVersionCreated => "spec_version_created",
            EventKind::SpecApproved => "spec_approved",
            EventKind::TaskReady => "task_ready",
            EventKind::RunStarted => "run_started",
            EventKind::RunBlocked => "run_blocked",
            EventKind::RunFinished => "run_finished",
            EventKind::ApprovalRequested => "approval_requested",
            EventKind::ApprovalGranted => "approval_granted",
            EventKind::ApprovalRejected => "approval_rejected",
            EventKind::PreviewReady => "preview_ready",
            EventKind::PreviewStale => "preview_stale",
            EventKind::PreviewError => "preview_error",
            EventKind::MessagePosted => "message_posted",
        }
    }
}

/// An event before persistence: no `seq` yet, the store assigns it.
/// `payload` is a JSON string built by the application layer — the domain
/// stays serialization-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub project_id: ProjectId,
    pub kind: EventKind,
    pub payload: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_stable_wire_name() {
        let kinds = [
            EventKind::SpecVersionCreated,
            EventKind::SpecApproved,
            EventKind::TaskReady,
            EventKind::RunStarted,
            EventKind::RunBlocked,
            EventKind::RunFinished,
            EventKind::ApprovalRequested,
            EventKind::ApprovalGranted,
            EventKind::ApprovalRejected,
            EventKind::PreviewReady,
            EventKind::PreviewStale,
            EventKind::PreviewError,
            EventKind::MessagePosted,
        ];
        for kind in kinds {
            assert!(!kind.as_str().is_empty());
            assert!(kind.as_str().chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }
}
