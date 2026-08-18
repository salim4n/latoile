//! A real Chromium renderer for approved architecture mockups. The adapter
//! launches an isolated profile, drives the Chrome DevTools Protocol, blocks
//! network URLs, disables motion, waits on the declared ready selector, and
//! captures PNG + deterministic DOM geometry + the browser accessibility tree.

mod artifacts;
mod cdp;
mod compare;
mod page;

use artifacts::{
    ComparisonArtifactBytes, artifact_dir, persist_artifacts, persist_comparison_artifacts,
    read_baseline_artifacts, read_comparison_png, sha256, sha256_file, verify_artifacts,
    verify_comparison_artifacts,
};
use cdp::{CdpClient, ChromeProcess, find_browser};
use latoile_core::ports::{
    PortError, PortResult, VisualBaselineRenderer, VisualComparisonRenderer,
};
use latoile_core::{
    CapturedVisualBaseline, CapturedVisualComparison, VisualBaseline, VisualBaselineCaptureOutcome,
    VisualBaselineCaptureRequest, VisualComparison, VisualComparisonCaptureOutcome,
    VisualComparisonCaptureRequest,
};
use page::{
    MAX_PNG_BYTES, apply_allowed_masks, capture_accessibility, capture_font_probe,
    capture_geometry, capture_png, configure_live_page, configure_page, settle_page,
    validate_request, wait_until_ready,
};
use serde_json::{Value, json};
use std::path::PathBuf;

const CAPTURE_PROTOCOL_VERSION: &str = "latoile-capture-v2";

#[derive(Debug, thiserror::Error)]
enum CaptureError {
    #[error("no supported Chromium executable was found")]
    BrowserUnavailable,
    #[error("the capture input is not a bounded self-contained HTML document")]
    UnsafeInput,
    #[error("the live capture URL is not a bounded loopback preview URL")]
    UnsafeUrl,
    #[error("Chromium did not expose its debugging endpoint in time")]
    BrowserStartup,
    #[error("the declared ready selector did not become uniquely available")]
    ReadinessTimeout,
    #[error("a stable selector is invalid, missing or resolves to more than one element")]
    UnstableSelector,
    #[error("the same immutable scenario produced different artifact bytes")]
    DeterminismMismatch,
    #[error("baseline evidence cannot be compared: {0}")]
    Evidence(String),
    #[error("live browser or font environment does not match the immutable baseline")]
    EnvironmentMismatch,
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
        wait_until_ready(&mut cdp, &request.scenario).await?;
        apply_allowed_masks(&mut cdp, &request.scenario).await?;
        let geometry = capture_geometry(&mut cdp, &request.scenario).await?;
        let accessibility = capture_accessibility(&mut cdp).await?;
        let font_probe = capture_font_probe(&mut cdp, &request.scenario).await?;
        let png = capture_png(&mut cdp, &request.scenario).await?;
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

    async fn compare_inner(
        &self,
        request: &VisualComparisonCaptureRequest,
    ) -> Result<CapturedVisualComparison, CaptureError> {
        if !request.baseline.satisfies(
            &request.spec_version_id,
            &request.manifest_digest,
            &request.package_commit_sha,
            &request.scenario.comparison_id,
        ) {
            return Err(CaptureError::Evidence(
                "the requested baseline does not match the immutable scenario".into(),
            ));
        }
        let baseline = read_baseline_artifacts(&self.root, &request.baseline)
            .map_err(|error| CaptureError::Evidence(error.to_string()))?;
        let baseline_environment: Value = serde_json::from_slice(&baseline.environment)
            .map_err(|error| CaptureError::Evidence(error.to_string()))?;
        if baseline_environment
            .get("protocol")
            .and_then(Value::as_str)
            != Some(CAPTURE_PROTOCOL_VERSION)
        {
            return Err(CaptureError::EnvironmentMismatch);
        }
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
        let live_url = configure_live_page(&mut cdp, request).await?;
        wait_until_ready(&mut cdp, &request.scenario).await?;
        settle_page(&mut cdp).await?;
        apply_allowed_masks(&mut cdp, &request.scenario).await?;
        let geometry = capture_geometry(&mut cdp, &request.scenario).await?;
        let accessibility = capture_accessibility(&mut cdp).await?;
        let font_probe = capture_font_probe(&mut cdp, &request.scenario).await?;
        let render_png = capture_png(&mut cdp, &request.scenario).await?;
        let browser_version = cdp.call("Browser.getVersion", json!({})).await?;
        browser.shutdown().await;

        let product = browser_version
            .get("product")
            .and_then(Value::as_str)
            .unwrap_or("Chromium/unknown")
            .to_string();
        let font_bytes = serde_json::to_vec(&font_probe)
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        let font_fingerprint = sha256(&font_bytes);
        if request.baseline.browser_version.as_deref() != Some(product.as_str())
            || request.baseline.font_fingerprint.as_deref() != Some(font_fingerprint.as_str())
        {
            return Err(CaptureError::EnvironmentMismatch);
        }
        let computed = compare::compare(&baseline, &render_png, &geometry, &accessibility)?;
        let environment = json!({
            "protocol": CAPTURE_PROTOCOL_VERSION,
            "kind": "live_comparison",
            "url": live_url,
            "network_policy": "exact_loopback_origin_only",
            "browser": browser_version,
            "browser_executable_sha256": executable_digest,
            "font_probe": font_probe,
            "baseline_environment_digest": request.baseline.environment_digest,
            "baseline_environment": baseline_environment,
            "locale": request.scenario.locale,
            "theme": request.scenario.theme,
            "fixture": request.scenario.fixture,
            "viewport": {
                "width": request.scenario.viewport_width,
                "height": request.scenario.viewport_height,
                "device_scale_factor_milli": request.scenario.device_scale_factor_milli,
            },
            "animations": "disabled",
            "process_environment": "cleared",
        });
        let environment = serde_json::to_vec(&environment)
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        let bytes = ComparisonArtifactBytes {
            render_png,
            pixel_diff_png: computed.pixel_diff_png,
            heatmap_png: computed.heatmap_png,
            geometry_diff: computed.geometry_diff,
            accessibility_diff: computed.accessibility_diff,
            environment,
        };
        let captured = CapturedVisualComparison {
            changed_pixels: computed.changed_pixels,
            total_pixels: computed.total_pixels,
            max_geometry_delta_milli: computed.max_geometry_delta_milli,
            accessibility_changes: computed.accessibility_changes,
            render_png_digest: sha256(&bytes.render_png),
            pixel_diff_digest: sha256(&bytes.pixel_diff_png),
            heatmap_png_digest: sha256(&bytes.heatmap_png),
            geometry_diff_digest: sha256(&bytes.geometry_diff),
            accessibility_diff_digest: sha256(&bytes.accessibility_diff),
            environment_digest: sha256(&bytes.environment),
            browser_version: product,
            font_fingerprint,
        };
        persist_comparison_artifacts(&self.root, request, &captured, &bytes)?;
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

impl VisualComparisonRenderer for BaselineCapture {
    async fn compare(
        &self,
        request: &VisualComparisonCaptureRequest,
    ) -> PortResult<VisualComparisonCaptureOutcome> {
        Ok(match self.compare_inner(request).await {
            Ok(captured) => VisualComparisonCaptureOutcome::Ready(captured),
            Err(error) => {
                let (code, recovery_action) = failure_details(&error);
                VisualComparisonCaptureOutcome::Invalid {
                    code: code.into(),
                    message: error.to_string(),
                    recovery_action: recovery_action.into(),
                }
            }
        })
    }

    async fn read_render_png(&self, comparison: &VisualComparison) -> PortResult<Vec<u8>> {
        let bytes = read_comparison_png(&self.root, comparison, "render.png")?;
        if bytes.len() > MAX_PNG_BYTES {
            return Err(PortError("render PNG exceeds the artifact limit".into()));
        }
        Ok(bytes)
    }

    async fn read_heatmap_png(&self, comparison: &VisualComparison) -> PortResult<Vec<u8>> {
        let bytes = read_comparison_png(&self.root, comparison, "heatmap.png")?;
        if bytes.len() > MAX_PNG_BYTES {
            return Err(PortError("heatmap PNG exceeds the artifact limit".into()));
        }
        Ok(bytes)
    }

    async fn verify_comparison(&self, comparison: &VisualComparison) -> PortResult<()> {
        verify_comparison_artifacts(&self.root, comparison)
    }
}

fn failure_outcome(error: CaptureError) -> VisualBaselineCaptureOutcome {
    let (code, recovery_action) = failure_details(&error);
    VisualBaselineCaptureOutcome::Failed {
        code: code.into(),
        message: error.to_string(),
        recovery_action: recovery_action.into(),
    }
}

fn failure_details(error: &CaptureError) -> (&'static str, &'static str) {
    match error {
        CaptureError::BrowserUnavailable | CaptureError::BrowserStartup => (
            "browser_unavailable",
            "Install a supported Chromium build or set LATOILE_CAPTURE_BROWSER, then retry.",
        ),
        CaptureError::UnsafeInput => (
            "unsafe_scenario",
            "Generate a new architecture version with a bounded self-contained mockup.",
        ),
        CaptureError::UnsafeUrl => (
            "unsafe_preview_url",
            "Restart the supervised loopback preview and retry the comparison.",
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
        CaptureError::Evidence(_) => (
            "invalid_baseline_evidence",
            "Re-capture or restore the immutable baseline artifacts before retrying.",
        ),
        CaptureError::EnvironmentMismatch => (
            "environment_mismatch",
            "Use the same capture protocol, Chromium and fonts; if the protocol changed, generate and approve a new architecture version.",
        ),
        CaptureError::Protocol(_) | CaptureError::Storage(_) => (
            "capture_failed",
            "Inspect the LaToile service logs, repair the capture runtime and retry.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use latoile_core::{
        ArchitectureVisualScenario, ProjectId, RunId, SpecVersionId, VisualComparisonId,
        VisualComparisonStatus,
    };
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

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

    #[tokio::test]
    #[ignore = "requires an installed Chromium browser"]
    async fn installed_chromium_passes_the_exact_mockup_served_over_http() {
        let root = tempfile::tempdir().unwrap();
        let capture = BaselineCapture::new(root.path().into());
        let html = "<!doctype html><html><head><style>html,body{margin:0}main{box-sizing:border-box;width:390px;height:844px;padding:24px;background:#fff;color:#111}</style></head><body><main data-latoile-ready='true'>Stable baseline</main></body></html>";
        let mut baseline_request = request(html);
        baseline_request.scenario.readiness_selector =
            "[data-latoile-ready='true']".into();
        let baseline_captured = match capture.capture(&baseline_request).await.unwrap() {
            VisualBaselineCaptureOutcome::Ready(captured) => captured,
            failure => panic!("baseline capture failed: {failure:?}"),
        };
        let baseline = VisualBaseline::ready(&baseline_request, &baseline_captured).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        let comparison_request = VisualComparisonCaptureRequest {
            id: VisualComparisonId::new("visual:run-exact:home-default").unwrap(),
            spec_version_id: baseline_request.spec_version_id.clone(),
            project_id: baseline_request.project_id.clone(),
            run_id: RunId::new("run-exact").unwrap(),
            manifest_digest: baseline_request.manifest_digest.clone(),
            package_commit_sha: baseline_request.package_commit_sha.clone(),
            baseline,
            scenario: baseline_request.scenario,
            live_base_url: format!("http://127.0.0.1:{port}"),
        };
        let captured = match capture.compare(&comparison_request).await.unwrap() {
            VisualComparisonCaptureOutcome::Ready(captured) => captured,
            failure => panic!("live comparison failed: {failure:?}"),
        };
        let comparison = VisualComparison::ready(&comparison_request, &captured).unwrap();
        assert_eq!(
            comparison.status,
            VisualComparisonStatus::Passed,
            "exact HTTP render drifted: changed_pixels={}, ratio_micros={}, geometry_delta_milli={}, accessibility_changes={}",
            comparison.changed_pixels,
            comparison.pixel_ratio_micros,
            comparison.max_geometry_delta_milli,
            comparison.accessibility_changes
        );
        server.abort();
    }

    #[tokio::test]
    #[ignore = "requires an installed Chromium browser"]
    async fn installed_chromium_detects_a_live_spacing_regression() {
        let root = tempfile::tempdir().unwrap();
        let capture = BaselineCapture::new(root.path().into());
        let mut baseline_request = request(
            "<!doctype html><html><head><style>html,body{margin:0}main{box-sizing:border-box;margin-left:20px;width:120px;height:120px;background:#181818;color:#fff}</style></head><body><main data-latoile-ready='true'>Card</main></body></html>",
        );
        baseline_request.scenario.readiness_selector = "[data-latoile-ready='true']".into();
        let baseline_captured = match capture.capture(&baseline_request).await.unwrap() {
            VisualBaselineCaptureOutcome::Ready(captured) => captured,
            failure => panic!("baseline capture failed: {failure:?}"),
        };
        let baseline = VisualBaseline::ready(&baseline_request, &baseline_captured).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let network_trap = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let trap_port = network_trap.local_addr().unwrap().port();
        let html = format!(
            "<!doctype html><html><head><style>html,body{{margin:0}}main{{box-sizing:border-box;margin-left:36px;width:120px;height:120px;background:#181818;color:#fff}}</style></head><body><main>Card</main><script>fetch('http://127.0.0.1:{trap_port}/must-not-connect').then(() => document.querySelector('main').textContent = 'NETWORK LEAK').catch(() => document.querySelector('main').dataset.latoileReady = 'true')</script></body></html>"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        let comparison_request = VisualComparisonCaptureRequest {
            id: VisualComparisonId::new("visual:run-1:home-default").unwrap(),
            spec_version_id: baseline_request.spec_version_id.clone(),
            project_id: baseline_request.project_id.clone(),
            run_id: RunId::new("run-1").unwrap(),
            manifest_digest: baseline_request.manifest_digest.clone(),
            package_commit_sha: baseline_request.package_commit_sha.clone(),
            baseline,
            scenario: baseline_request.scenario,
            live_base_url: format!("http://127.0.0.1:{port}"),
        };
        let captured = match capture.compare(&comparison_request).await.unwrap() {
            VisualComparisonCaptureOutcome::Ready(captured) => captured,
            failure => panic!("live comparison failed: {failure:?}"),
        };
        let comparison = VisualComparison::ready(&comparison_request, &captured).unwrap();
        assert_eq!(comparison.status, VisualComparisonStatus::Blocking);
        assert!(comparison.changed_pixels > 0);
        assert_eq!(comparison.max_geometry_delta_milli, 16_000);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), network_trap.accept())
                .await
                .is_err(),
            "the live page reached a non-approved loopback origin"
        );
        server.abort();
    }
}
