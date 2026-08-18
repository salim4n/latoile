//! One cohesive trusted-visual-evidence aggregate: an approved baseline and
//! every live comparison share the same scenario/provenance chain, digest
//! validation and fail-closed semantics. The domain stores only bounded facts;
//! PNG and snapshot bytes remain in the capture adapter's immutable store.

use crate::architecture::ArchitectureVisualScenario;
use crate::error::DomainError;
use crate::ids::{ProjectId, RunId, SpecVersionId, VisualComparisonId};

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

/// Server-owned verdict for one baseline/live-render comparison. These
/// thresholds are product policy, never values supplied by an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualComparisonStatus {
    Invalid,
    Blocking,
    Reservation,
    Passed,
}

impl VisualComparisonStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::Blocking => "blocking",
            Self::Reservation => "reservation",
            Self::Passed => "passed",
        }
    }

    pub fn has_trusted_evidence(self) -> bool {
        self != Self::Invalid
    }
}

pub const BLOCKING_PIXEL_RATIO_MICROS: u32 = 20_000;
pub const RESERVATION_PIXEL_RATIO_MICROS: u32 = 2_000;
pub const BLOCKING_GEOMETRY_DELTA_MILLI: u32 = 8_000;
pub const BLOCKING_ACCESSIBILITY_CHANGES: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualComparison {
    pub id: VisualComparisonId,
    pub spec_version_id: SpecVersionId,
    pub project_id: ProjectId,
    pub run_id: RunId,
    pub comparison_id: String,
    pub manifest_digest: String,
    pub package_commit_sha: String,
    pub baseline_png_digest: String,
    pub status: VisualComparisonStatus,
    pub changed_pixels: u64,
    pub total_pixels: u64,
    pub pixel_ratio_micros: u32,
    pub max_geometry_delta_milli: u32,
    pub accessibility_changes: u32,
    pub render_png_digest: Option<String>,
    pub pixel_diff_digest: Option<String>,
    pub heatmap_png_digest: Option<String>,
    pub geometry_diff_digest: Option<String>,
    pub accessibility_diff_digest: Option<String>,
    pub environment_digest: Option<String>,
    pub browser_version: Option<String>,
    pub font_fingerprint: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub recovery_action: Option<String>,
}

impl VisualComparison {
    pub fn ready(
        request: &VisualComparisonCaptureRequest,
        captured: &CapturedVisualComparison,
    ) -> Result<Self, DomainError> {
        if !request.baseline.satisfies(
            &request.spec_version_id,
            &request.manifest_digest,
            &request.package_commit_sha,
            &request.scenario.comparison_id,
        ) {
            return Err(DomainError::Invariant(
                "a visual comparison requires the matching immutable baseline",
            ));
        }
        for digest in [
            &request.manifest_digest,
            &captured.render_png_digest,
            &captured.pixel_diff_digest,
            &captured.heatmap_png_digest,
            &captured.geometry_diff_digest,
            &captured.accessibility_diff_digest,
            &captured.environment_digest,
            &captured.font_fingerprint,
        ] {
            require_sha256(digest)?;
        }
        if captured.total_pixels == 0 || captured.changed_pixels > captured.total_pixels {
            return Err(DomainError::Invariant(
                "visual comparison pixel counts are invalid",
            ));
        }
        if captured.browser_version.trim().is_empty() {
            return Err(DomainError::Invariant(
                "a visual comparison must record its browser version",
            ));
        }
        let ratio = captured
            .changed_pixels
            .saturating_mul(1_000_000)
            .checked_div(captured.total_pixels)
            .unwrap_or(1_000_000)
            .min(1_000_000) as u32;
        let status = if ratio >= BLOCKING_PIXEL_RATIO_MICROS
            || captured.max_geometry_delta_milli >= BLOCKING_GEOMETRY_DELTA_MILLI
            || captured.accessibility_changes >= BLOCKING_ACCESSIBILITY_CHANGES
        {
            VisualComparisonStatus::Blocking
        } else if ratio >= RESERVATION_PIXEL_RATIO_MICROS
            || captured.max_geometry_delta_milli > 0
            || captured.accessibility_changes > 0
        {
            VisualComparisonStatus::Reservation
        } else {
            VisualComparisonStatus::Passed
        };
        Ok(Self {
            id: request.id.clone(),
            spec_version_id: request.spec_version_id.clone(),
            project_id: request.project_id.clone(),
            run_id: request.run_id.clone(),
            comparison_id: request.scenario.comparison_id.clone(),
            manifest_digest: request.manifest_digest.clone(),
            package_commit_sha: request.package_commit_sha.clone(),
            baseline_png_digest: request
                .baseline
                .png_digest
                .clone()
                .ok_or(DomainError::Invariant("baseline PNG digest is missing"))?,
            status,
            changed_pixels: captured.changed_pixels,
            total_pixels: captured.total_pixels,
            pixel_ratio_micros: ratio,
            max_geometry_delta_milli: captured.max_geometry_delta_milli,
            accessibility_changes: captured.accessibility_changes,
            render_png_digest: Some(captured.render_png_digest.clone()),
            pixel_diff_digest: Some(captured.pixel_diff_digest.clone()),
            heatmap_png_digest: Some(captured.heatmap_png_digest.clone()),
            geometry_diff_digest: Some(captured.geometry_diff_digest.clone()),
            accessibility_diff_digest: Some(captured.accessibility_diff_digest.clone()),
            environment_digest: Some(captured.environment_digest.clone()),
            browser_version: Some(captured.browser_version.clone()),
            font_fingerprint: Some(captured.font_fingerprint.clone()),
            failure_code: None,
            failure_message: None,
            recovery_action: None,
        })
    }

    pub fn invalid(
        request: &VisualComparisonCaptureRequest,
        code: impl Into<String>,
        message: impl Into<String>,
        recovery_action: impl Into<String>,
    ) -> Result<Self, DomainError> {
        require_sha256(&request.manifest_digest)?;
        let baseline_png_digest = request
            .baseline
            .png_digest
            .clone()
            .ok_or(DomainError::Invariant("baseline PNG digest is missing"))?;
        let code = code.into();
        let message = message.into();
        let recovery_action = recovery_action.into();
        if code.trim().is_empty() || message.trim().is_empty() || recovery_action.trim().is_empty()
        {
            return Err(DomainError::Invariant(
                "an invalid visual comparison needs a code, explanation and recovery action",
            ));
        }
        Ok(Self {
            id: request.id.clone(),
            spec_version_id: request.spec_version_id.clone(),
            project_id: request.project_id.clone(),
            run_id: request.run_id.clone(),
            comparison_id: request.scenario.comparison_id.clone(),
            manifest_digest: request.manifest_digest.clone(),
            package_commit_sha: request.package_commit_sha.clone(),
            baseline_png_digest,
            status: VisualComparisonStatus::Invalid,
            changed_pixels: 0,
            total_pixels: 0,
            pixel_ratio_micros: 0,
            max_geometry_delta_milli: 0,
            accessibility_changes: 0,
            render_png_digest: None,
            pixel_diff_digest: None,
            heatmap_png_digest: None,
            geometry_diff_digest: None,
            accessibility_diff_digest: None,
            environment_digest: None,
            browser_version: None,
            font_fingerprint: None,
            failure_code: Some(code),
            failure_message: Some(message),
            recovery_action: Some(recovery_action),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualComparisonCaptureRequest {
    pub id: VisualComparisonId,
    pub spec_version_id: SpecVersionId,
    pub project_id: ProjectId,
    pub run_id: RunId,
    pub manifest_digest: String,
    pub package_commit_sha: String,
    pub baseline: VisualBaseline,
    pub scenario: ArchitectureVisualScenario,
    pub live_base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedVisualComparison {
    pub changed_pixels: u64,
    pub total_pixels: u64,
    pub max_geometry_delta_milli: u32,
    pub accessibility_changes: u32,
    pub render_png_digest: String,
    pub pixel_diff_digest: String,
    pub heatmap_png_digest: String,
    pub geometry_diff_digest: String,
    pub accessibility_diff_digest: String,
    pub environment_digest: String,
    pub browser_version: String,
    pub font_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualComparisonCaptureOutcome {
    Ready(CapturedVisualComparison),
    Invalid {
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

    fn comparison_request() -> VisualComparisonCaptureRequest {
        let baseline_capture = CapturedVisualBaseline {
            png_digest: "b".repeat(64),
            geometry_digest: "c".repeat(64),
            accessibility_digest: "d".repeat(64),
            environment_digest: "e".repeat(64),
            browser_version: "Chrome/151.0.0.0".into(),
            font_fingerprint: "f".repeat(64),
        };
        let baseline_request = request();
        VisualComparisonCaptureRequest {
            id: VisualComparisonId::new("vc-1").unwrap(),
            spec_version_id: baseline_request.spec_version_id.clone(),
            project_id: baseline_request.project_id.clone(),
            run_id: RunId::new("run-1").unwrap(),
            manifest_digest: baseline_request.manifest_digest.clone(),
            package_commit_sha: baseline_request.package_commit_sha.clone(),
            baseline: VisualBaseline::ready(&baseline_request, &baseline_capture).unwrap(),
            scenario: baseline_request.scenario,
            live_base_url: "http://127.0.0.1:4100".into(),
        }
    }

    fn captured_comparison() -> CapturedVisualComparison {
        CapturedVisualComparison {
            changed_pixels: 0,
            total_pixels: 100_000,
            max_geometry_delta_milli: 0,
            accessibility_changes: 0,
            render_png_digest: "1".repeat(64),
            pixel_diff_digest: "2".repeat(64),
            heatmap_png_digest: "3".repeat(64),
            geometry_diff_digest: "4".repeat(64),
            accessibility_diff_digest: "5".repeat(64),
            environment_digest: "6".repeat(64),
            browser_version: "Chrome/151.0.0.0".into(),
            font_fingerprint: "7".repeat(64),
        }
    }

    #[test]
    fn comparison_thresholds_are_server_owned_and_deterministic() {
        let request = comparison_request();
        let mut captured = captured_comparison();
        assert_eq!(
            VisualComparison::ready(&request, &captured).unwrap().status,
            VisualComparisonStatus::Passed
        );

        captured.changed_pixels = 300;
        assert_eq!(
            VisualComparison::ready(&request, &captured).unwrap().status,
            VisualComparisonStatus::Reservation
        );

        captured.changed_pixels = 2_000;
        assert_eq!(
            VisualComparison::ready(&request, &captured).unwrap().status,
            VisualComparisonStatus::Blocking
        );
        captured.changed_pixels = 0;
        captured.max_geometry_delta_milli = 16_000;
        assert_eq!(
            VisualComparison::ready(&request, &captured).unwrap().status,
            VisualComparisonStatus::Blocking
        );
    }

    #[test]
    fn invalid_comparison_is_actionable_and_has_no_fabricated_metrics() {
        let comparison = VisualComparison::invalid(
            &comparison_request(),
            "readiness_timeout",
            "The live route never became ready.",
            "Fix the route and rerun the frontend task.",
        )
        .unwrap();
        assert_eq!(comparison.status, VisualComparisonStatus::Invalid);
        assert_eq!(comparison.total_pixels, 0);
        assert!(comparison.render_png_digest.is_none());
    }
}
