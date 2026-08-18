//! Persistent Socratic architecture discovery. The owner still speaks only
//! through the Manager surface, but the Architect owns a dedicated session
//! and an auditable sequence of questions. Draft generation is deliberately
//! a later transition: discovery must finish before files may be written.

use crate::error::{DomainError, TransitionError};
use crate::ids::{ArchitectureQuestionId, ArchitectureSessionId, ProjectId};

pub const ARCHITECT_SKILL_NAME: &str = "app-architect-brainstorm";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchitectureOperatingMode {
    Greenfield,
    ReverseEngineering,
}

impl ArchitectureOperatingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Greenfield => "greenfield",
            Self::ReverseEngineering => "reverse_engineering",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchitecturePackageStatus {
    NotStarted,
    Generating,
    DraftReady,
}

impl ArchitecturePackageStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Generating => "generating",
            Self::DraftReady => "draft_ready",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitecturePackageEvidence {
    pub design_dir: String,
    pub base_sha: String,
    pub head_sha: String,
    pub tree_sha: String,
    pub package_digest: String,
    pub manifest_digest: String,
    pub changed_files: Vec<String>,
    pub diff_stat: String,
}

/// One deterministic visual contract declared by the architecture manifest.
/// `comparison_id` is the stable key reused by baseline capture and pixel
/// comparison; viewport and locale are data, never conventions hidden in a
/// browser script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitectureVisualScenario {
    pub comparison_id: String,
    pub screen: String,
    pub state: String,
    pub locale: String,
    pub theme: String,
    /// Live application route reused by run comparison. Baseline capture
    /// reads `mockup`; both targets share every other scenario field.
    pub route: String,
    /// Stable synthetic data set name. Real customer data is never needed to
    /// reproduce visual evidence.
    pub fixture: String,
    pub readiness_selector: String,
    pub stable_selectors: Vec<String>,
    pub allowed_masks: Vec<String>,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub device_scale_factor_milli: u32,
    pub mockup: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitectureValidationFinding {
    pub code: String,
    pub message: String,
}

/// Read-only proof produced from Git plus the package bytes. Invalid packages
/// are values (with findings), not adapter failures, so the owner can inspect
/// exactly why approval is blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitecturePackageValidation {
    pub valid: bool,
    pub package_digest: String,
    pub manifest_digest: String,
    pub commit_sha: String,
    pub tree_sha: String,
    pub file_count: u32,
    pub gallery_path: String,
    pub scenarios: Vec<ArchitectureVisualScenario>,
    pub findings: Vec<ArchitectureValidationFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArchitecturePhase {
    DomainDiscovery,
    Requirements,
    UxDiscovery,
    ReadyToDraft,
}

impl ArchitecturePhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DomainDiscovery => "domain_discovery",
            Self::Requirements => "requirements",
            Self::UxDiscovery => "ux_discovery",
            Self::ReadyToDraft => "ready_to_draft",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchitectureStatus {
    Discovering,
    AwaitingAnswer,
    ReadyToDraft,
    Failed,
    Cancelled,
}

impl ArchitectureStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovering => "discovering",
            Self::AwaitingAnswer => "awaiting_answer",
            Self::ReadyToDraft => "ready_to_draft",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Discovering | Self::AwaitingAnswer | Self::ReadyToDraft
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitectureSession {
    pub id: ArchitectureSessionId,
    pub project_id: ProjectId,
    pub status: ArchitectureStatus,
    pub phase: ArchitecturePhase,
    pub acp_session_id: Option<String>,
    pub skill_name: Option<String>,
    pub skill_digest: Option<String>,
    pub operating_mode: Option<ArchitectureOperatingMode>,
    pub package_status: ArchitecturePackageStatus,
    pub package: Option<ArchitecturePackageEvidence>,
    pub failure_reason: Option<String>,
}

impl ArchitectureSession {
    pub fn new(id: ArchitectureSessionId, project_id: ProjectId) -> Self {
        Self {
            id,
            project_id,
            status: ArchitectureStatus::Discovering,
            phase: ArchitecturePhase::DomainDiscovery,
            acp_session_id: None,
            skill_name: None,
            skill_digest: None,
            operating_mode: None,
            package_status: ArchitecturePackageStatus::NotStarted,
            package: None,
            failure_reason: None,
        }
    }

    pub fn attach_agent(&mut self, session_id: impl Into<String>) -> Result<(), DomainError> {
        if self.status != ArchitectureStatus::Discovering || self.acp_session_id.is_some() {
            return Err(DomainError::Invariant(
                "an Architect session attaches exactly once while discovery starts",
            ));
        }
        let session_id = session_id.into();
        if session_id.trim().is_empty() {
            return Err(DomainError::Invariant(
                "an Architect session id cannot be blank",
            ));
        }
        self.acp_session_id = Some(session_id);
        Ok(())
    }

    pub fn record_skill(
        &mut self,
        name: impl Into<String>,
        digest: impl Into<String>,
        mode: ArchitectureOperatingMode,
    ) -> Result<(), DomainError> {
        if self.status != ArchitectureStatus::Discovering
            || self.skill_name.is_some()
            || self.skill_digest.is_some()
            || self.operating_mode.is_some()
        {
            return Err(DomainError::Invariant(
                "an Architect skill identity is recorded exactly once when discovery starts",
            ));
        }
        let name = name.into();
        let digest = digest.into();
        if name != ARCHITECT_SKILL_NAME {
            return Err(DomainError::Invariant(
                "architecture discovery requires app-architect-brainstorm",
            ));
        }
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainError::Invariant(
                "an Architect skill digest must be a SHA-256 hex digest",
            ));
        }
        self.skill_name = Some(name);
        self.skill_digest = Some(digest.to_ascii_lowercase());
        self.operating_mode = Some(mode);
        Ok(())
    }

    pub fn ask(&mut self, phase: ArchitecturePhase) -> Result<(), DomainError> {
        if self.status != ArchitectureStatus::Discovering {
            return Err(TransitionError::new(
                "architecture_session",
                self.status.as_str(),
                ArchitectureStatus::AwaitingAnswer.as_str(),
            )
            .into());
        }
        if phase < self.phase || phase == ArchitecturePhase::ReadyToDraft {
            return Err(DomainError::Invariant(
                "architecture discovery phases move forward and questions precede drafting",
            ));
        }
        self.phase = phase;
        self.status = ArchitectureStatus::AwaitingAnswer;
        Ok(())
    }

    pub fn receive_answer(&mut self) -> Result<(), DomainError> {
        if self.status != ArchitectureStatus::AwaitingAnswer {
            return Err(TransitionError::new(
                "architecture_session",
                self.status.as_str(),
                ArchitectureStatus::Discovering.as_str(),
            )
            .into());
        }
        self.status = ArchitectureStatus::Discovering;
        Ok(())
    }

    pub fn ready_to_draft(&mut self) -> Result<(), DomainError> {
        if self.status != ArchitectureStatus::Discovering {
            return Err(TransitionError::new(
                "architecture_session",
                self.status.as_str(),
                ArchitectureStatus::ReadyToDraft.as_str(),
            )
            .into());
        }
        self.phase = ArchitecturePhase::ReadyToDraft;
        self.status = ArchitectureStatus::ReadyToDraft;
        Ok(())
    }

    pub fn begin_package(&mut self) -> Result<(), DomainError> {
        if self.status != ArchitectureStatus::ReadyToDraft
            || self.package_status != ArchitecturePackageStatus::NotStarted
            || self.skill_name.is_none()
            || self.skill_digest.is_none()
            || self.operating_mode.is_none()
        {
            return Err(DomainError::Invariant(
                "an architecture package starts once, after discovery and with pinned skill provenance",
            ));
        }
        self.package_status = ArchitecturePackageStatus::Generating;
        Ok(())
    }

    pub fn finish_package(
        &mut self,
        evidence: ArchitecturePackageEvidence,
    ) -> Result<(), DomainError> {
        if self.status != ArchitectureStatus::ReadyToDraft
            || self.package_status != ArchitecturePackageStatus::Generating
        {
            return Err(DomainError::Invariant(
                "an architecture package can finish only from its generating state",
            ));
        }
        if evidence.design_dir.trim().is_empty()
            || evidence.base_sha.trim().is_empty()
            || evidence.head_sha.trim().is_empty()
            || evidence.tree_sha.trim().is_empty()
            || evidence.package_digest.len() != 64
            || evidence.changed_files.is_empty()
            || evidence
                .changed_files
                .iter()
                .any(|path| !path.starts_with(&evidence.design_dir))
        {
            return Err(DomainError::Invariant(
                "architecture package evidence must be complete and confined to its design directory",
            ));
        }
        self.package = Some(evidence);
        self.package_status = ArchitecturePackageStatus::DraftReady;
        Ok(())
    }

    pub fn needs_live_process(&self) -> bool {
        matches!(
            (self.status, self.package_status),
            (ArchitectureStatus::Discovering, _)
                | (ArchitectureStatus::AwaitingAnswer, _)
                | (
                    ArchitectureStatus::ReadyToDraft,
                    ArchitecturePackageStatus::NotStarted
                )
                | (
                    ArchitectureStatus::ReadyToDraft,
                    ArchitecturePackageStatus::Generating
                )
        )
    }

    pub fn fail(&mut self, reason: impl Into<String>) -> Result<(), DomainError> {
        if !self.status.is_active() {
            return Err(TransitionError::new(
                "architecture_session",
                self.status.as_str(),
                ArchitectureStatus::Failed.as_str(),
            )
            .into());
        }
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(DomainError::Invariant(
                "a failed architecture session needs an actionable reason",
            ));
        }
        self.status = ArchitectureStatus::Failed;
        self.failure_reason = Some(reason);
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), DomainError> {
        if !self.status.is_active() {
            return Err(TransitionError::new(
                "architecture_session",
                self.status.as_str(),
                ArchitectureStatus::Cancelled.as_str(),
            )
            .into());
        }
        self.status = ArchitectureStatus::Cancelled;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchitectureQuestionStatus {
    Open,
    Answered,
}

impl ArchitectureQuestionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Answered => "answered",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitectureQuestion {
    pub id: ArchitectureQuestionId,
    pub session_id: ArchitectureSessionId,
    pub sequence: u32,
    pub prompt: String,
    pub status: ArchitectureQuestionStatus,
    pub answer: Option<String>,
}

impl ArchitectureQuestion {
    pub fn new(
        id: ArchitectureQuestionId,
        session_id: ArchitectureSessionId,
        sequence: u32,
        prompt: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let prompt = prompt.into();
        if prompt.trim().is_empty() {
            return Err(DomainError::Invariant(
                "an Architect question cannot be blank",
            ));
        }
        Ok(Self {
            id,
            session_id,
            sequence,
            prompt,
            status: ArchitectureQuestionStatus::Open,
            answer: None,
        })
    }

    pub fn answer(&mut self, answer: impl Into<String>) -> Result<(), DomainError> {
        if self.status != ArchitectureQuestionStatus::Open {
            return Err(TransitionError::new(
                "architecture_question",
                self.status.as_str(),
                ArchitectureQuestionStatus::Answered.as_str(),
            )
            .into());
        }
        let answer = answer.into();
        if answer.trim().is_empty() {
            return Err(DomainError::Invariant(
                "an Architect answer cannot be blank",
            ));
        }
        self.answer = Some(answer);
        self.status = ArchitectureQuestionStatus::Answered;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> ArchitectureSession {
        ArchitectureSession::new(
            ArchitectureSessionId::new("as1").unwrap(),
            ProjectId::new("p1").unwrap(),
        )
    }

    #[test]
    fn discovery_questions_move_forward_and_finish_before_drafting() {
        let mut session = session();
        session.attach_agent("acp:as1").unwrap();
        session
            .record_skill(
                ARCHITECT_SKILL_NAME,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ArchitectureOperatingMode::Greenfield,
            )
            .unwrap();
        session.ask(ArchitecturePhase::DomainDiscovery).unwrap();
        assert!(session.ready_to_draft().is_err());
        session.receive_answer().unwrap();
        session.ask(ArchitecturePhase::Requirements).unwrap();
        session.receive_answer().unwrap();
        session.ready_to_draft().unwrap();
        assert_eq!(session.status, ArchitectureStatus::ReadyToDraft);
        assert!(session.ask(ArchitecturePhase::UxDiscovery).is_err());

        session.begin_package().unwrap();
        session
            .finish_package(ArchitecturePackageEvidence {
                design_dir: "design/v1/".into(),
                base_sha: "base".into(),
                head_sha: "head".into(),
                tree_sha: "tree".into(),
                package_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .into(),
                manifest_digest: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .into(),
                changed_files: vec!["design/v1/architecture-spec.md".into()],
                diff_stat: "1 file changed".into(),
            })
            .unwrap();
        assert_eq!(
            session.package_status,
            ArchitecturePackageStatus::DraftReady
        );
        assert!(!session.needs_live_process());
    }

    #[test]
    fn phases_never_regress() {
        let mut session = session();
        session.ask(ArchitecturePhase::UxDiscovery).unwrap();
        session.receive_answer().unwrap();
        assert!(session.ask(ArchitecturePhase::Requirements).is_err());
    }

    #[test]
    fn failure_and_cancellation_are_terminal_and_actionable() {
        let mut failed = session();
        assert!(failed.fail(" ").is_err());
        failed.fail("provider session lost after restart").unwrap();
        assert!(failed.cancel().is_err());

        let mut cancelled = session();
        cancelled.cancel().unwrap();
        assert!(cancelled.ready_to_draft().is_err());
    }

    #[test]
    fn a_question_is_answered_exactly_once() {
        let mut question = ArchitectureQuestion::new(
            ArchitectureQuestionId::new("aq1").unwrap(),
            ArchitectureSessionId::new("as1").unwrap(),
            1,
            "Qui utilise le produit ?",
        )
        .unwrap();
        assert!(question.answer(" ").is_err());
        question.answer("Une équipe produit").unwrap();
        assert!(question.answer("Deuxième réponse").is_err());
    }
}
