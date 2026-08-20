//! Atomic, content-addressed storage for immutable visual evidence.

use crate::CaptureError;
use latoile_core::ports::{PortError, PortResult};
use latoile_core::{
    CapturedVisualBaseline, CapturedVisualComparison, VisualBaseline, VisualBaselineCaptureRequest,
    VisualComparison, VisualComparisonCaptureRequest,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(super) struct BaselineArtifacts {
    pub png: Vec<u8>,
    pub geometry: Vec<u8>,
    pub accessibility: Vec<u8>,
    pub environment: Vec<u8>,
}

pub(super) struct ComparisonArtifactBytes {
    pub render_png: Vec<u8>,
    pub pixel_diff_png: Vec<u8>,
    pub heatmap_png: Vec<u8>,
    pub geometry_diff: Vec<u8>,
    pub accessibility_diff: Vec<u8>,
    pub environment: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn persist_artifacts(
    root: &Path,
    request: &VisualBaselineCaptureRequest,
    captured: &CapturedVisualBaseline,
    png: &[u8],
    geometry: &[u8],
    accessibility: &[u8],
    environment: &[u8],
) -> Result<(), CaptureError> {
    if sha256(png) != captured.png_digest
        || sha256(geometry) != captured.geometry_digest
        || sha256(accessibility) != captured.accessibility_digest
        || sha256(environment) != captured.environment_digest
    {
        return Err(CaptureError::DeterminismMismatch);
    }
    std::fs::create_dir_all(root).map_err(|error| CaptureError::Storage(error.to_string()))?;
    let final_dir = artifact_dir(
        root,
        request.spec_version_id.as_str(),
        &request.scenario.comparison_id,
    );
    if final_dir.exists() {
        let checks = [
            ("baseline.png", captured.png_digest.as_str()),
            ("geometry.json", captured.geometry_digest.as_str()),
            ("accessibility.json", captured.accessibility_digest.as_str()),
            ("environment.json", captured.environment_digest.as_str()),
        ];
        if checks.iter().all(|(name, digest)| {
            std::fs::read(final_dir.join(name))
                .map(|bytes| sha256(&bytes) == *digest)
                .unwrap_or(false)
        }) {
            return Ok(());
        }
        return Err(CaptureError::DeterminismMismatch);
    }
    let parent = final_dir
        .parent()
        .ok_or_else(|| CaptureError::Storage("artifact parent is missing".into()))?;
    std::fs::create_dir_all(parent).map_err(|error| CaptureError::Storage(error.to_string()))?;
    let temp = tempfile::Builder::new()
        .prefix(".capture-")
        .tempdir_in(parent)
        .map_err(|error| CaptureError::Storage(error.to_string()))?;
    write_synced(&temp.path().join("baseline.png"), png)?;
    write_synced(&temp.path().join("geometry.json"), geometry)?;
    write_synced(&temp.path().join("accessibility.json"), accessibility)?;
    write_synced(&temp.path().join("environment.json"), environment)?;
    let metadata = serde_json::to_vec(&json!({
        "spec_version_id": request.spec_version_id.as_str(),
        "project_id": request.project_id.as_str(),
        "comparison_id": request.scenario.comparison_id,
        "manifest_digest": request.manifest_digest,
        "package_commit_sha": request.package_commit_sha,
        "png_digest": captured.png_digest,
        "geometry_digest": captured.geometry_digest,
        "accessibility_digest": captured.accessibility_digest,
        "environment_digest": captured.environment_digest,
        "browser_version": captured.browser_version,
        "font_fingerprint": captured.font_fingerprint,
    }))
    .map_err(|error| CaptureError::Storage(error.to_string()))?;
    write_synced(&temp.path().join("metadata.json"), &metadata)?;
    let temp_path = temp.keep();
    std::fs::rename(temp_path, &final_dir)
        .map_err(|error| CaptureError::Storage(error.to_string()))?;
    Ok(())
}

pub(super) fn verify_artifacts(root: &Path, baseline: &VisualBaseline) -> PortResult<()> {
    let dir = artifact_dir(
        root,
        baseline.spec_version_id.as_str(),
        &baseline.comparison_id,
    );
    let expected = [
        ("baseline.png", baseline.png_digest.as_deref()),
        ("geometry.json", baseline.geometry_digest.as_deref()),
        (
            "accessibility.json",
            baseline.accessibility_digest.as_deref(),
        ),
        ("environment.json", baseline.environment_digest.as_deref()),
    ];
    for (name, digest) in expected {
        let digest = digest.ok_or_else(|| PortError("baseline evidence is incomplete".into()))?;
        let bytes = std::fs::read(dir.join(name))
            .map_err(|_| PortError("baseline artifact is missing".into()))?;
        if sha256(&bytes) != digest {
            return Err(PortError(
                "baseline artifact failed its content hash".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn read_baseline_artifacts(
    root: &Path,
    baseline: &VisualBaseline,
) -> PortResult<BaselineArtifacts> {
    verify_artifacts(root, baseline)?;
    let dir = artifact_dir(
        root,
        baseline.spec_version_id.as_str(),
        &baseline.comparison_id,
    );
    let read = |name: &str| {
        std::fs::read(dir.join(name))
            .map_err(|_| PortError(format!("baseline artifact {name} is missing")))
    };
    Ok(BaselineArtifacts {
        png: read("baseline.png")?,
        geometry: read("geometry.json")?,
        accessibility: read("accessibility.json")?,
        environment: read("environment.json")?,
    })
}

pub(super) fn persist_comparison_artifacts(
    root: &Path,
    request: &VisualComparisonCaptureRequest,
    captured: &CapturedVisualComparison,
    bytes: &ComparisonArtifactBytes,
) -> Result<(), CaptureError> {
    let checks = [
        (&bytes.render_png[..], captured.render_png_digest.as_str()),
        (
            &bytes.pixel_diff_png[..],
            captured.pixel_diff_digest.as_str(),
        ),
        (&bytes.heatmap_png[..], captured.heatmap_png_digest.as_str()),
        (
            &bytes.geometry_diff[..],
            captured.geometry_diff_digest.as_str(),
        ),
        (
            &bytes.accessibility_diff[..],
            captured.accessibility_diff_digest.as_str(),
        ),
        (&bytes.environment[..], captured.environment_digest.as_str()),
    ];
    if checks
        .iter()
        .any(|(content, digest)| sha256(content) != *digest)
    {
        return Err(CaptureError::DeterminismMismatch);
    }
    let final_dir = comparison_dir(root, request.id.as_str());
    let files = [
        (
            "render.png",
            &bytes.render_png,
            captured.render_png_digest.as_str(),
        ),
        (
            "pixel-diff.png",
            &bytes.pixel_diff_png,
            captured.pixel_diff_digest.as_str(),
        ),
        (
            "heatmap.png",
            &bytes.heatmap_png,
            captured.heatmap_png_digest.as_str(),
        ),
        (
            "geometry-diff.json",
            &bytes.geometry_diff,
            captured.geometry_diff_digest.as_str(),
        ),
        (
            "accessibility-diff.json",
            &bytes.accessibility_diff,
            captured.accessibility_diff_digest.as_str(),
        ),
        (
            "environment.json",
            &bytes.environment,
            captured.environment_digest.as_str(),
        ),
    ];
    if final_dir.exists() {
        if files.iter().all(|(name, _, digest)| {
            std::fs::read(final_dir.join(name))
                .map(|content| sha256(content) == *digest)
                .unwrap_or(false)
        }) {
            return Ok(());
        }
        return Err(CaptureError::DeterminismMismatch);
    }
    let parent = final_dir
        .parent()
        .ok_or_else(|| CaptureError::Storage("comparison artifact parent is missing".into()))?;
    std::fs::create_dir_all(parent).map_err(|error| CaptureError::Storage(error.to_string()))?;
    let temp = tempfile::Builder::new()
        .prefix(".comparison-")
        .tempdir_in(parent)
        .map_err(|error| CaptureError::Storage(error.to_string()))?;
    for (name, content, _) in files {
        write_synced(&temp.path().join(name), content)?;
    }
    let metadata = serde_json::to_vec(&json!({
        "id": request.id.as_str(),
        "spec_version_id": request.spec_version_id.as_str(),
        "project_id": request.project_id.as_str(),
        "run_id": request.run_id.as_str(),
        "comparison_id": request.scenario.comparison_id,
        "manifest_digest": request.manifest_digest,
        "package_commit_sha": request.package_commit_sha,
        "baseline_png_digest": request.baseline.png_digest,
        "changed_pixels": captured.changed_pixels,
        "total_pixels": captured.total_pixels,
        "max_geometry_delta_milli": captured.max_geometry_delta_milli,
        "accessibility_changes": captured.accessibility_changes,
        "render_png_digest": captured.render_png_digest,
        "pixel_diff_digest": captured.pixel_diff_digest,
        "heatmap_png_digest": captured.heatmap_png_digest,
        "geometry_diff_digest": captured.geometry_diff_digest,
        "accessibility_diff_digest": captured.accessibility_diff_digest,
        "environment_digest": captured.environment_digest,
        "browser_version": captured.browser_version,
        "font_fingerprint": captured.font_fingerprint,
    }))
    .map_err(|error| CaptureError::Storage(error.to_string()))?;
    write_synced(&temp.path().join("metadata.json"), &metadata)?;
    std::fs::rename(temp.keep(), &final_dir)
        .map_err(|error| CaptureError::Storage(error.to_string()))?;
    Ok(())
}

pub(super) fn verify_comparison_artifacts(
    root: &Path,
    comparison: &VisualComparison,
) -> PortResult<()> {
    if !comparison.status.has_trusted_evidence() {
        return Err(PortError(
            "invalid comparison has no trusted artifacts".into(),
        ));
    }
    let dir = comparison_dir(root, comparison.id.as_str());
    let expected = [
        ("render.png", comparison.render_png_digest.as_deref()),
        ("pixel-diff.png", comparison.pixel_diff_digest.as_deref()),
        ("heatmap.png", comparison.heatmap_png_digest.as_deref()),
        (
            "geometry-diff.json",
            comparison.geometry_diff_digest.as_deref(),
        ),
        (
            "accessibility-diff.json",
            comparison.accessibility_diff_digest.as_deref(),
        ),
        ("environment.json", comparison.environment_digest.as_deref()),
    ];
    for (name, digest) in expected {
        let digest = digest.ok_or_else(|| PortError("comparison evidence is incomplete".into()))?;
        let content = std::fs::read(dir.join(name))
            .map_err(|_| PortError(format!("comparison artifact {name} is missing")))?;
        if sha256(content) != digest {
            return Err(PortError(
                "comparison artifact failed its content hash".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn read_comparison_png(
    root: &Path,
    comparison: &VisualComparison,
    name: &str,
) -> PortResult<Vec<u8>> {
    verify_comparison_artifacts(root, comparison)?;
    let expected = match name {
        "render.png" => comparison.render_png_digest.as_deref(),
        "heatmap.png" => comparison.heatmap_png_digest.as_deref(),
        _ => None,
    }
    .ok_or_else(|| PortError("comparison PNG digest is missing".into()))?;
    let bytes = std::fs::read(comparison_dir(root, comparison.id.as_str()).join(name))
        .map_err(|_| PortError(format!("comparison artifact {name} is missing")))?;
    if sha256(&bytes) != expected {
        return Err(PortError("comparison PNG failed its content hash".into()));
    }
    Ok(bytes)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), CaptureError> {
    use std::io::Write;
    let mut file =
        std::fs::File::create(path).map_err(|error| CaptureError::Storage(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| CaptureError::Storage(error.to_string()))
}

pub(super) fn artifact_dir(root: &Path, spec: &str, comparison_id: &str) -> PathBuf {
    let key = sha256(format!("{spec}\0{comparison_id}").as_bytes());
    root.join(&key[..2]).join(key)
}

fn comparison_dir(root: &Path, id: &str) -> PathBuf {
    let key = sha256(id.as_bytes());
    root.join("comparisons").join(&key[..2]).join(key)
}

pub(super) fn sha256(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

pub(super) fn sha256_file(path: &Path) -> Result<String, CaptureError> {
    let bytes = std::fs::read(path).map_err(|error| CaptureError::Storage(error.to_string()))?;
    Ok(sha256(bytes))
}
