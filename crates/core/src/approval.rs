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
    /// The owner's immutable audit note attached to the decision.
    pub decision_comment: Option<String>,
    /// For a rejected review, the one corrective executor run it spawned.
    pub corrective_run_id: Option<RunId>,
}

impl Approval {
    pub fn new(id: ApprovalId, run_id: RunId, kind: ApprovalKind, payload: String) -> Self {
        Self {
            id,
            run_id,
            kind,
            status: ApprovalStatus::Pending,
            payload,
            decision_comment: None,
            corrective_run_id: None,
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
        self.grant_with_comment(None)
    }

    pub fn reject(&mut self) -> Result<(), DomainError> {
        self.reject_with_comment(None)
    }

    pub fn grant_with_comment(&mut self, comment: Option<String>) -> Result<(), DomainError> {
        let comment = clean_comment(comment)?;
        self.decide(ApprovalStatus::Granted)?;
        self.decision_comment = comment;
        Ok(())
    }

    pub fn reject_with_comment(&mut self, comment: Option<String>) -> Result<(), DomainError> {
        if self.kind == ApprovalKind::Review
            && comment.as_deref().is_none_or(|value| value.trim().is_empty())
        {
            return Err(DomainError::Invariant(
                "requesting changes requires an owner comment",
            ));
        }
        let comment = clean_comment(comment)?;
        self.decide(ApprovalStatus::Rejected)?;
        self.decision_comment = comment;
        Ok(())
    }

    pub fn attach_corrective_run(&mut self, run: RunId) -> Result<(), DomainError> {
        if self.kind != ApprovalKind::Review || self.status != ApprovalStatus::Rejected {
            return Err(DomainError::Invariant(
                "a corrective run belongs to a rejected review",
            ));
        }
        if self.corrective_run_id.is_some() {
            return Err(DomainError::Invariant(
                "a rejected review has only one corrective run",
            ));
        }
        self.corrective_run_id = Some(run);
        Ok(())
    }

    pub fn is_granted(&self) -> bool {
        self.status == ApprovalStatus::Granted
    }
}

fn clean_comment(comment: Option<String>) -> Result<Option<String>, DomainError> {
    let cleaned = comment.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    });
    if cleaned.as_ref().is_some_and(|value| value.len() > 8 * 1024) {
        return Err(DomainError::Invariant("approval comment is too long"));
    }
    Ok(cleaned)
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
        b.reject_with_comment(Some("Pas nécessaire".into())).unwrap();
        assert!(!b.is_granted());
        assert_eq!(b.decision_comment.as_deref(), Some("Pas nécessaire"));
        assert!(b.grant().is_err());
    }

    #[test]
    fn a_review_rejection_needs_a_comment_and_one_corrective_run() {
        let mut approval = approval(ApprovalKind::Review);
        assert!(approval.reject_with_comment(None).is_err());
        assert_eq!(approval.status, ApprovalStatus::Pending);

        approval
            .reject_with_comment(Some("  Corriger le focus clavier.  ".into()))
            .unwrap();
        assert_eq!(
            approval.decision_comment.as_deref(),
            Some("Corriger le focus clavier.")
        );
        approval
            .attach_corrective_run(RunId::new("correction-1").unwrap())
            .unwrap();
        assert!(approval
            .attach_corrective_run(RunId::new("correction-2").unwrap())
            .is_err());
    }
}
