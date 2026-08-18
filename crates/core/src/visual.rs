//! Trusted visual baselines. The domain stores only bounded provenance and
//! cryptographic hashes; PNG and snapshot bytes remain in the capture
//! adapter's immutable artifact store.

use crate::architecture::ArchitectureVisualScenario;
use crate::error::DomainError;
use crate::ids::{ProjectId, SpecVersionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualBaselineStatus {
    Ready,
    Failed,
}

impl VisualBaselineStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualBaseline {
    pub spec_version_id: SpecVersionId,
    pub project_id: ProjectId,
    pub comparison_id: String,
    pub manifest_digest: String,
    pub package_commit_sha: String,
    pub status: VisualBaselineStatus,
    pub png_digest: Option<String>,
    pub geometry_digest: Option<String>,
    pub accessibility_digest: Option<String>,
    pub environment_digest: Option<String>,
    pub browser_version: Option<String>,
    pub font_fingerprint: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub recovery_action: Option<String>,
}

impl VisualBaseline {
    pub fn ready(
        request: &VisualBaselineCaptureRequest,
        captured: &CapturedVisualBaseline,
    ) -> Result<Self, DomainError> {
        for digest in [
            &request.manifest_digest,
            &captured.png_digest,
            &captured.geometry_digest,
            &captured.accessibility_digest,
            &captured.environment_digest,
            &captured.font_fingerprint,
        ] {
            require_sha256(digest)?;
        }
        if captured.browser_version.trim().is_empty() {
            return Err(DomainError::Invariant(
                "a visual baseline must pin its browser version",
            ));
        }
        Ok(Self {
            spec_version_id: request.spec_version_id.clone(),
            project_id: request.project_id.clone(),
            comparison_id: request.scenario.comparison_id.clone(),
            manifest_digest: request.manifest_digest.clone(),
            package_commit_sha: request.package_commit_sha.clone(),
            status: VisualBaselineStatus::Ready,
            png_digest: Some(captured.png_digest.clone()),
            geometry_digest: Some(captured.geometry_digest.clone()),
            accessibility_digest: Some(captured.accessibility_digest.clone()),
            environment_digest: Some(captured.environment_digest.clone()),
            browser_version: Some(captured.browser_version.clone()),
            font_fingerprint: Some(captured.font_fingerprint.clone()),
            failure_code: None,
            failure_message: None,
            recovery_action: None,
        })
    }

    pub fn failed(
        request: &VisualBaselineCaptureRequest,
        code: impl Into<String>,
        message: impl Into<String>,
        recovery_action: impl Into<String>,
    ) -> Result<Self, DomainError> {
        require_sha256(&request.manifest_digest)?;
        let code = code.into();
        let message = message.into();
        let recovery_action = recovery_action.into();
        if code.trim().is_empty() || message.trim().is_empty() || recovery_action.trim().is_empty()
        {
            return Err(DomainError::Invariant(
                "a failed visual baseline needs a code, explanation and recovery action",
            ));
        }
        Ok(Self {
            spec_version_id: request.spec_version_id.clone(),
            project_id: request.project_id.clone(),
            comparison_id: request.scenario.comparison_id.clone(),
            manifest_digest: request.manifest_digest.clone(),
            package_commit_sha: request.package_commit_sha.clone(),
            status: VisualBaselineStatus::Failed,
            png_digest: None,
            geometry_digest: None,
            accessibility_digest: None,
            environment_digest: None,
            browser_version: None,
            font_fingerprint: None,
            failure_code: Some(code),
            failure_message: Some(message),
            recovery_action: Some(recovery_action),
        })
    }

    pub fn satisfies(
        &self,
        spec_version_id: &SpecVersionId,
        manifest_digest: &str,
        package_commit_sha: &str,
        comparison_id: &str,
    ) -> bool {
        self.status == VisualBaselineStatus::Ready
            && &self.spec_version_id == spec_version_id
            && self.manifest_digest == manifest_digest
            && self.package_commit_sha == package_commit_sha
            && self.comparison_id == comparison_id
            && self.png_digest.is_some()
            && self.geometry_digest.is_some()
            && self.accessibility_digest.is_some()
            && self.environment_digest.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualBaselineCaptureRequest {
    pub spec_version_id: SpecVersionId,
    pub project_id: ProjectId,
    pub manifest_digest: String,
    pub package_commit_sha: String,
    pub scenario: ArchitectureVisualScenario,
    pub html: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedVisualBaseline {
    pub png_digest: String,
    pub geometry_digest: String,
    pub accessibility_digest: String,
    pub environment_digest: String,
    pub browser_version: String,
    pub font_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualBaselineCaptureOutcome {
    Ready(CapturedVisualBaseline),
    Failed {
        code: String,
        message: String,
        recovery_action: String,
    },
}

fn require_sha256(value: &str) -> Result<(), DomainError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DomainError::Invariant(
            "visual evidence digests must be lowercase SHA-256 hex",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> VisualBaselineCaptureRequest {
        VisualBaselineCaptureRequest {
            spec_version_id: SpecVersionId::new("spec-1").unwrap(),
            project_id: ProjectId::new("project-1").unwrap(),
            manifest_digest: "a".repeat(64),
            package_commit_sha: "1".repeat(40),
            scenario: ArchitectureVisualScenario {
                comparison_id: "home-default-fr-mobile".into(),
                screen: "home".into(),
                state: "default".into(),
                locale: "fr-FR".into(),
                theme: "light".into(),
                route: "/".into(),
                fixture: "synthetic-default".into(),
                readiness_selector: "[data-latoile-ready='true']".into(),
                stable_selectors: vec!["main".into()],
                allowed_masks: Vec::new(),
                viewport_width: 390,
                viewport_height: 844,
                device_scale_factor_milli: 1000,
                mockup: "mockups/home.html".into(),
            },
            html: "<!doctype html>".into(),
        }
    }

    #[test]
    fn ready_baseline_requires_complete_cryptographic_evidence() {
        let captured = CapturedVisualBaseline {
            png_digest: "b".repeat(64),
            geometry_digest: "c".repeat(64),
            accessibility_digest: "d".repeat(64),
            environment_digest: "e".repeat(64),
            browser_version: "Chrome/151.0.0.0".into(),
            font_fingerprint: "f".repeat(64),
        };
        let baseline = VisualBaseline::ready(&request(), &captured).unwrap();
        assert_eq!(baseline.status, VisualBaselineStatus::Ready);
        assert!(baseline.satisfies(
            &request().spec_version_id,
            &request().manifest_digest,
            &request().package_commit_sha,
            "home-default-fr-mobile",
        ));
    }

    #[test]
    fn failed_baseline_is_actionable_and_never_ready() {
        let baseline = VisualBaseline::failed(
            &request(),
            "readiness_timeout",
            "The ready selector was not visible.",
            "Fix the mockup selector and generate a new spec version.",
        )
        .unwrap();
        assert_eq!(baseline.status, VisualBaselineStatus::Failed);
        assert!(baseline.png_digest.is_none());
    }
}
