//! A real Chromium renderer for approved architecture mockups. The adapter
//! launches an isolated profile, drives the Chrome DevTools Protocol, blocks
//! network URLs, disables motion, waits on the declared ready selector, and
//! captures PNG + deterministic DOM geometry + the browser accessibility tree.

mod artifacts;
mod cdp;
mod page;

use artifacts::{artifact_dir, persist_artifacts, sha256, sha256_file, verify_artifacts};
use cdp::{CdpClient, ChromeProcess, find_browser};
use latoile_core::ports::{PortError, PortResult, VisualBaselineRenderer};
use latoile_core::{
    CapturedVisualBaseline, VisualBaseline, VisualBaselineCaptureOutcome,
    VisualBaselineCaptureRequest,
};
use page::{
    MAX_PNG_BYTES, apply_allowed_masks, capture_accessibility, capture_font_probe,
    capture_geometry, capture_png, configure_page, validate_request, wait_until_ready,
};
use serde_json::{Value, json};
use std::path::PathBuf;

const CAPTURE_PROTOCOL_VERSION: &str = "latoile-capture-v1";

#[derive(Debug, thiserror::Error)]
enum CaptureError {
    #[error("no supported Chromium executable was found")]
    BrowserUnavailable,
    #[error("the capture input is not a bounded self-contained HTML document")]
    UnsafeInput,
    #[error("Chromium did not expose its debugging endpoint in time")]
    BrowserStartup,
    #[error("the declared ready selector did not become uniquely available")]
    ReadinessTimeout,
    #[error("a stable selector is invalid, missing or resolves to more than one element")]
    UnstableSelector,
    #[error("the same immutable scenario produced different artifact bytes")]
    DeterminismMismatch,
    #[error("browser protocol failure: {0}")]
    Protocol(String),
    #[error("visual artifact storage failure: {0}")]
    Storage(String),
}

#[derive(Clone)]
pub struct BaselineCapture {
    root: PathBuf,
    browser_override: Option<PathBuf>,
}

impl BaselineCapture {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            browser_override: std::env::var_os("LATOILE_CAPTURE_BROWSER").map(PathBuf::from),
        }
    }

    #[cfg(test)]
    fn with_browser(root: PathBuf, browser: Option<PathBuf>) -> Self {
        Self {
            root,
            browser_override: browser,
        }
    }

    async fn capture_inner(
        &self,
        request: &VisualBaselineCaptureRequest,
    ) -> Result<CapturedVisualBaseline, CaptureError> {
        validate_request(request)?;
        let executable = find_browser(self.browser_override.as_deref())?;
        let executable_digest = tokio::task::spawn_blocking({
            let executable = executable.clone();
            move || sha256_file(&executable)
        })
        .await
        .map_err(|error| CaptureError::Storage(error.to_string()))??;

        let mut browser = ChromeProcess::launch(&executable).await?;
        let page_ws = browser.page_websocket().await?;
        let mut cdp = CdpClient::connect(&page_ws).await?;
        configure_page(&mut cdp, request).await?;
        wait_until_ready(&mut cdp, request).await?;

        let geometry = capture_geometry(&mut cdp, request).await?;
        let accessibility = capture_accessibility(&mut cdp).await?;
        let font_probe = capture_font_probe(&mut cdp, request).await?;
        apply_allowed_masks(&mut cdp, request).await?;
        let png = capture_png(&mut cdp, request).await?;
        let browser_version = cdp.call("Browser.getVersion", json!({})).await?;
        browser.shutdown().await;

        let geometry_bytes = serde_json::to_vec(&geometry)
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        let accessibility_bytes = serde_json::to_vec(&accessibility)
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        let font_bytes = serde_json::to_vec(&font_probe)
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        let environment = json!({
            "protocol": CAPTURE_PROTOCOL_VERSION,
            "browser": browser_version,
            "browser_executable_sha256": executable_digest,
            "font_probe": font_probe,
            "locale": request.scenario.locale,
            "theme": request.scenario.theme,
            "viewport": {
                "width": request.scenario.viewport_width,
                "height": request.scenario.viewport_height,
                "device_scale_factor_milli": request.scenario.device_scale_factor_milli,
            },
            "animations": "disabled",
            "network": "blocked",
        });
        let environment_bytes = serde_json::to_vec(&environment)
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        let product = browser_version
            .get("product")
            .and_then(Value::as_str)
            .unwrap_or("Chromium/unknown")
            .to_string();
        let captured = CapturedVisualBaseline {
            png_digest: sha256(&png),
            geometry_digest: sha256(&geometry_bytes),
            accessibility_digest: sha256(&accessibility_bytes),
            environment_digest: sha256(&environment_bytes),
            browser_version: product,
            font_fingerprint: sha256(&font_bytes),
        };
        persist_artifacts(
            &self.root,
            request,
            &captured,
            &png,
            &geometry_bytes,
            &accessibility_bytes,
            &environment_bytes,
        )?;
        Ok(captured)
    }
}

impl VisualBaselineRenderer for BaselineCapture {
    async fn capture(
        &self,
        request: &VisualBaselineCaptureRequest,
    ) -> PortResult<VisualBaselineCaptureOutcome> {
        Ok(match self.capture_inner(request).await {
            Ok(captured) => VisualBaselineCaptureOutcome::Ready(captured),
            Err(error) => failure_outcome(error),
        })
    }

    async fn read_png(&self, baseline: &VisualBaseline) -> PortResult<Vec<u8>> {
        verify_artifacts(&self.root, baseline)?;
        let expected = baseline
            .png_digest
            .as_deref()
            .ok_or_else(|| PortError("failed baseline has no PNG".into()))?;
        let path = artifact_dir(
            &self.root,
            baseline.spec_version_id.as_str(),
            &baseline.comparison_id,
        )
        .join("baseline.png");
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| PortError("baseline PNG is missing".into()))?;
        if bytes.len() > MAX_PNG_BYTES || sha256(&bytes) != expected {
            return Err(PortError("baseline PNG failed its content hash".into()));
        }
        Ok(bytes)
    }

    async fn verify(&self, baseline: &VisualBaseline) -> PortResult<()> {
        verify_artifacts(&self.root, baseline)
    }
}

fn failure_outcome(error: CaptureError) -> VisualBaselineCaptureOutcome {
    let (code, recovery_action) = match error {
        CaptureError::BrowserUnavailable | CaptureError::BrowserStartup => (
            "browser_unavailable",
            "Install a supported Chromium build or set LATOILE_CAPTURE_BROWSER, then retry.",
        ),
        CaptureError::UnsafeInput => (
            "unsafe_scenario",
            "Generate a new architecture version with a bounded self-contained mockup.",
        ),
        CaptureError::ReadinessTimeout => (
            "readiness_timeout",
            "Fix the declared readiness selector in a new architecture version, then retry.",
        ),
        CaptureError::UnstableSelector => (
            "unstable_selector",
            "Give every measured selector exactly one stable element in a new architecture version.",
        ),
        CaptureError::DeterminismMismatch => (
            "determinism_mismatch",
            "Pin the changed browser/font environment or remove nondeterminism, then generate a new version.",
        ),
        CaptureError::Protocol(_) | CaptureError::Storage(_) => (
            "capture_failed",
            "Inspect the LaToile service logs, repair the capture runtime and retry.",
        ),
    };
    VisualBaselineCaptureOutcome::Failed {
        code: code.into(),
        message: error.to_string(),
        recovery_action: recovery_action.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use latoile_core::{ArchitectureVisualScenario, ProjectId, SpecVersionId};

    fn request(html: &str) -> VisualBaselineCaptureRequest {
        VisualBaselineCaptureRequest {
            spec_version_id: SpecVersionId::new("spec-1").unwrap(),
            project_id: ProjectId::new("project-1").unwrap(),
            manifest_digest: "a".repeat(64),
            package_commit_sha: "1".repeat(40),
            scenario: ArchitectureVisualScenario {
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
                mockup: "mockups/home.html".into(),
            },
            html: html.into(),
        }
    }

    fn captured() -> CapturedVisualBaseline {
        CapturedVisualBaseline {
            png_digest: sha256(b"png"),
            geometry_digest: sha256(b"geometry"),
            accessibility_digest: sha256(b"accessibility"),
            environment_digest: sha256(b"environment"),
            browser_version: "Chrome/151".into(),
            font_fingerprint: sha256(b"fonts"),
        }
    }

    #[tokio::test]
    async fn unsafe_html_is_refused_before_browser_discovery() {
        let root = tempfile::tempdir().unwrap();
        let capture = BaselineCapture::with_browser(root.path().into(), None);
        let outcome = capture
            .capture(&request("<html><script>fetch('https://x')</script></html>"))
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            VisualBaselineCaptureOutcome::Failed { ref code, .. } if code == "unsafe_scenario"
        ));
    }

    #[test]
    fn identical_repeat_is_idempotent_and_changed_bytes_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let request = request("<html><main>Stable</main></html>");
        let captured = captured();
        persist_artifacts(
            root.path(),
            &request,
            &captured,
            b"png",
            b"geometry",
            b"accessibility",
            b"environment",
        )
        .unwrap();
        persist_artifacts(
            root.path(),
            &request,
            &captured,
            b"png",
            b"geometry",
            b"accessibility",
            b"environment",
        )
        .unwrap();
        assert!(matches!(
            persist_artifacts(
                root.path(),
                &request,
                &captured,
                b"changed",
                b"geometry",
                b"accessibility",
                b"environment",
            ),
            Err(CaptureError::DeterminismMismatch)
        ));
    }

    #[tokio::test]
    #[ignore = "requires an installed Chromium browser"]
    async fn installed_chromium_produces_a_real_repeatable_png_and_browser_tree() {
        let root = tempfile::tempdir().unwrap();
        let capture = BaselineCapture::new(root.path().into());
        let mut request = request(
            "<!doctype html><html><head><style>html,body{margin:0}main{box-sizing:border-box;width:390px;height:844px;padding:24px;background:#fff;color:#111}</style></head><body><main>Stable baseline</main></body></html>",
        );
        request.scenario.device_scale_factor_milli = 2000;
        let first = capture.capture(&request).await.unwrap();
        let second = capture.capture(&request).await.unwrap();
        assert_eq!(first, second);
        assert!(matches!(first, VisualBaselineCaptureOutcome::Ready(_)));
    }
}
