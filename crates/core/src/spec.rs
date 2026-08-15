//! `SpecVersion` — a versioned, approved specification. Artifacts live in the
//! project repo under `design_dir` (ADR-003); this entity tracks metadata and
//! status only. One approved spec per project — the application layer
//! supersedes the previous one when a new draft is approved.

use crate::error::{DomainError, TransitionError};
use crate::ids::{ProjectId, RunId, SpecVersionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecStatus {
    Draft,
    Approved,
    Superseded,
}

impl SpecStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecStatus::Draft => "draft",
            SpecStatus::Approved => "approved",
            SpecStatus::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecVersion {
    pub id: SpecVersionId,
    pub project_id: ProjectId,
    pub version: u32,
    pub status: SpecStatus,
    pub design_dir: String,
    pub architect_run_id: Option<RunId>,
}

impl SpecVersion {
    pub fn new(
        id: SpecVersionId,
        project_id: ProjectId,
        version: u32,
        design_dir: impl Into<String>,
        architect_run_id: Option<RunId>,
    ) -> Result<Self, DomainError> {
        let design_dir = design_dir.into();
        if design_dir.trim().is_empty() {
            return Err(DomainError::Invariant("a spec version needs a design dir"));
        }
        Ok(Self {
            id,
            project_id,
            version,
            status: SpecStatus::Draft,
            design_dir,
            architect_run_id,
        })
    }

    /// The owner approves this spec. Only a draft can be approved; approving
    /// a second draft supersedes the previously approved one (handled by the
    /// application layer, which owns the cross-entity rule).
    pub fn approve(&mut self) -> Result<(), DomainError> {
        if self.status != SpecStatus::Draft {
            return Err(
                TransitionError::new("spec_version", self.status.as_str(), "approved").into(),
            );
        }
        self.status = SpecStatus::Approved;
        Ok(())
    }

    pub fn supersede(&mut self) -> Result<(), DomainError> {
        if self.status != SpecStatus::Approved {
            return Err(
                TransitionError::new("spec_version", self.status.as_str(), "superseded").into(),
            );
        }
        self.status = SpecStatus::Superseded;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SpecVersion {
        SpecVersion::new(
            SpecVersionId::new("s1").unwrap(),
            ProjectId::new("p1").unwrap(),
            1,
            "design/",
            None,
        )
        .unwrap()
    }

    #[test]
    fn draft_approve_supersede_chain() {
        let mut s = spec();
        assert!(s.supersede().is_err()); // Draft cannot be superseded
        s.approve().unwrap();
        assert!(s.approve().is_err()); // Approved cannot be re-approved
        s.supersede().unwrap();
        assert!(s.approve().is_err()); // Superseded is terminal
    }
}
