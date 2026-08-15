//! `Approval` — the human's decision point. Three kinds: spec validation,
//! review verdict, permission grant. Pending is the only state that can
//! change; granted and rejected are terminal.

use crate::error::{DomainError, TransitionError};
use crate::ids::{ApprovalId, RunId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalKind {
    Spec,
    Review,
    Permission,
}

impl ApprovalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalKind::Spec => "spec",
            ApprovalKind::Review => "review",
            ApprovalKind::Permission => "permission",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    Pending,
    Granted,
    Rejected,
}

impl ApprovalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalStatus::Pending => "pending",
            ApprovalStatus::Granted => "granted",
            ApprovalStatus::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub id: ApprovalId,
    pub run_id: RunId,
    pub kind: ApprovalKind,
    pub status: ApprovalStatus,
    /// JSON: diff reference, reviewer findings, or the permission request.
    pub payload: String,
}

impl Approval {
    pub fn new(id: ApprovalId, run_id: RunId, kind: ApprovalKind, payload: String) -> Self {
        Self {
            id,
            run_id,
            kind,
            status: ApprovalStatus::Pending,
            payload,
        }
    }

    fn decide(&mut self, to: ApprovalStatus) -> Result<(), DomainError> {
        if self.status != ApprovalStatus::Pending {
            return Err(
                TransitionError::new("approval", self.status.as_str(), to.as_str()).into(),
            );
        }
        self.status = to;
        Ok(())
    }

    pub fn grant(&mut self) -> Result<(), DomainError> {
        self.decide(ApprovalStatus::Granted)
    }

    pub fn reject(&mut self) -> Result<(), DomainError> {
        self.decide(ApprovalStatus::Rejected)
    }

    pub fn is_granted(&self) -> bool {
        self.status == ApprovalStatus::Granted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(kind: ApprovalKind) -> Approval {
        Approval::new(
            ApprovalId::new("a1").unwrap(),
            RunId::new("r1").unwrap(),
            kind,
            "{}".into(),
        )
    }

    #[test]
    fn pending_can_be_granted_or_rejected_once() {
        let mut a = approval(ApprovalKind::Review);
        a.grant().unwrap();
        assert!(a.is_granted());
        assert!(a.grant().is_err()); // decided is terminal
        assert!(a.reject().is_err());

        let mut b = approval(ApprovalKind::Permission);
        b.reject().unwrap();
        assert!(!b.is_granted());
        assert!(b.grant().is_err());
    }
}
