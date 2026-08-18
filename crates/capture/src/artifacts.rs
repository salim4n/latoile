//! Atomic, content-addressed storage for immutable visual evidence.

use crate::CaptureError;
use latoile_core::ports::{PortError, PortResult};
use latoile_core::{CapturedVisualBaseline, VisualBaseline, VisualBaselineCaptureRequest};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

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

pub(super) fn sha256(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

pub(super) fn sha256_file(path: &Path) -> Result<String, CaptureError> {
    let bytes = std::fs::read(path).map_err(|error| CaptureError::Storage(error.to_string()))?;
    Ok(sha256(bytes))
}
