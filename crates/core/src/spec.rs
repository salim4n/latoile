//! `SpecVersion` — a versioned, approved specification. Artifacts live in the
//! project repo under `design_dir` (ADR-003); this entity tracks metadata and
//! status only. One approved spec per project — the application layer
//! supersedes the previous one when a new draft is approved.

use crate::architecture::{ArchitectureOperatingMode, ArchitecturePackageValidation};
use crate::error::{DomainError, TransitionError};
use crate::ids::{ArchitectureSessionId, ProjectId, RunId, SpecVersionId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecProvenance {
    pub architecture_session_id: ArchitectureSessionId,
    pub skill_name: String,
    pub skill_digest: String,
    pub operating_mode: ArchitectureOperatingMode,
    pub package_digest: String,
    pub manifest_digest: String,
    pub package_commit_sha: String,
    pub package_tree_sha: String,
}

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
    pub provenance: Option<SpecProvenance>,
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
            provenance: None,
        })
    }

    pub fn attach_provenance(&mut self, provenance: SpecProvenance) -> Result<(), DomainError> {
        if self.status != SpecStatus::Draft || self.provenance.is_some() {
            return Err(DomainError::Invariant(
                "architecture provenance attaches exactly once to a draft",
            ));
        }
        if provenance.skill_name.trim().is_empty()
            || !is_hex_digest(&provenance.skill_digest, &[64])
            || !is_hex_digest(&provenance.package_digest, &[64])
            || !is_hex_digest(&provenance.manifest_digest, &[64])
            || !is_hex_digest(&provenance.package_commit_sha, &[40, 64])
            || !is_hex_digest(&provenance.package_tree_sha, &[40, 64])
        {
            return Err(DomainError::Invariant(
                "spec provenance must pin the skill and immutable package",
            ));
        }
        self.provenance = Some(provenance);
        Ok(())
    }

    /// The owner approves this spec. Only a draft can be approved; approving
    /// a second draft supersedes the previously approved one (handled by the
    /// application layer, which owns the cross-entity rule).
    pub fn approve(
        &mut self,
        verification: &ArchitecturePackageValidation,
    ) -> Result<(), DomainError> {
        if self.status != SpecStatus::Draft {
            return Err(
                TransitionError::new("spec_version", self.status.as_str(), "approved").into(),
            );
        }
        let provenance = self.provenance.as_ref().ok_or(DomainError::Invariant(
            "only a verified architecture package can be approved",
        ))?;
        if !verification.valid
            || verification.package_digest != provenance.package_digest
            || verification.manifest_digest != provenance.manifest_digest
            || verification.commit_sha != provenance.package_commit_sha
            || verification.tree_sha != provenance.package_tree_sha
            || verification.scenarios.is_empty()
        {
            return Err(DomainError::Invariant(
                "architecture approval proof does not match the immutable draft",
            ));
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

fn is_hex_digest(value: &str, allowed_lengths: &[usize]) -> bool {
    allowed_lengths.contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> SpecProvenance {
        SpecProvenance {
            architecture_session_id: ArchitectureSessionId::new("architecture-1").unwrap(),
            skill_name: "app-architect-brainstorm".into(),
            skill_digest: "a".repeat(64),
            operating_mode: ArchitectureOperatingMode::Greenfield,
            package_digest: "b".repeat(64),
            manifest_digest: "c".repeat(64),
            package_commit_sha: "1".repeat(40),
            package_tree_sha: "2".repeat(40),
        }
    }

    fn verification(provenance: &SpecProvenance) -> ArchitecturePackageValidation {
        ArchitecturePackageValidation {
            valid: true,
            package_digest: provenance.package_digest.clone(),
            manifest_digest: provenance.manifest_digest.clone(),
            commit_sha: provenance.package_commit_sha.clone(),
            tree_sha: provenance.package_tree_sha.clone(),
            file_count: 16,
            gallery_path: "gallery.html".into(),
            scenarios: vec![crate::ArchitectureVisualScenario {
                comparison_id: "home-default".into(),
                screen: "home".into(),
                state: "default".into(),
                locale: "fr-FR".into(),
                theme: "light".into(),
                route: "/".into(),
                fixture: "synthetic-default".into(),
                readiness_selector: "main".into(),
                stable_selectors: vec!["main".into()],
                allowed_masks: Vec::new(),
                viewport_width: 390,
                viewport_height: 844,
                device_scale_factor_milli: 1000,
                mockup: "mockups/home-default.html".into(),
            }],
            findings: Vec::new(),
        }
    }

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
        let provenance = provenance();
        let verification = verification(&provenance);
        s.attach_provenance(provenance).unwrap();
        s.approve(&verification).unwrap();
        assert!(s.approve(&verification).is_err()); // Approved cannot be re-approved
        s.supersede().unwrap();
        assert!(s.approve(&verification).is_err()); // Superseded is terminal
    }

    #[test]
    fn approval_rejects_missing_or_mismatched_immutable_proof() {
        let mut missing = spec();
        let provenance = provenance();
        assert!(missing.approve(&verification(&provenance)).is_err());

        let mut mismatched = spec();
        mismatched.attach_provenance(provenance.clone()).unwrap();
        let mut wrong = verification(&provenance);
        wrong.package_digest = "d".repeat(64);
        assert!(mismatched.approve(&wrong).is_err());
        assert_eq!(mismatched.status, SpecStatus::Draft);
    }
}
