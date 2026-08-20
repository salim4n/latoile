//! Bounded metadata for immutable browser baselines. Successful evidence can
//! never be rewritten; a failed attempt may be retried for the same spec and
//! scenario contract.

use super::{Store, StoreError, unknown_variant};
use latoile_core::ids::{ProjectId, SpecVersionId};
use latoile_core::ports::{PortResult, VisualBaselineStore};
use latoile_core::{VisualBaseline, VisualBaselineStatus};
use sqlx::Row;

fn row_to_baseline(row: &sqlx::sqlite::SqliteRow) -> Result<VisualBaseline, StoreError> {
    let status = match row.try_get::<String, _>("status")?.as_str() {
        "ready" => VisualBaselineStatus::Ready,
        "failed" => VisualBaselineStatus::Failed,
        raw => return Err(unknown_variant("visual baseline status", raw)),
    };
    Ok(VisualBaseline {
        spec_version_id: SpecVersionId::new(row.try_get::<String, _>("spec_version_id")?)?,
        project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)?,
        comparison_id: row.try_get("comparison_id")?,
        manifest_digest: row.try_get("manifest_digest")?,
        package_commit_sha: row.try_get("package_commit_sha")?,
        status,
        png_digest: row.try_get("png_digest")?,
        geometry_digest: row.try_get("geometry_digest")?,
        accessibility_digest: row.try_get("accessibility_digest")?,
        environment_digest: row.try_get("environment_digest")?,
        browser_version: row.try_get("browser_version")?,
        font_fingerprint: row.try_get("font_fingerprint")?,
        failure_code: row.try_get("failure_code")?,
        failure_message: row.try_get("failure_message")?,
        recovery_action: row.try_get("recovery_action")?,
    })
}

impl VisualBaselineStore for Store {
    async fn get(
        &self,
        spec: &SpecVersionId,
        comparison_id: &str,
    ) -> PortResult<Option<VisualBaseline>> {
        let row = sqlx::query(
            "SELECT * FROM visual_baseline WHERE spec_version_id = ? AND comparison_id = ?",
        )
        .bind(spec.as_str())
        .bind(comparison_id)
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.as_ref()
            .map(row_to_baseline)
            .transpose()
            .map_err(Into::into)
    }

    async fn list_for_spec(&self, spec: &SpecVersionId) -> PortResult<Vec<VisualBaseline>> {
        let rows = sqlx::query(
            "SELECT * FROM visual_baseline WHERE spec_version_id = ? ORDER BY comparison_id",
        )
        .bind(spec.as_str())
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_baseline)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn save(&self, baseline: &VisualBaseline) -> PortResult<()> {
        let result = sqlx::query(
            "INSERT INTO visual_baseline (
                spec_version_id, project_id, comparison_id, manifest_digest,
                package_commit_sha, status, png_digest, geometry_digest,
                accessibility_digest, environment_digest, browser_version,
                font_fingerprint, failure_code, failure_message, recovery_action
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(spec_version_id, comparison_id) DO UPDATE SET
                status = excluded.status,
                png_digest = excluded.png_digest,
                geometry_digest = excluded.geometry_digest,
                accessibility_digest = excluded.accessibility_digest,
                environment_digest = excluded.environment_digest,
                browser_version = excluded.browser_version,
                font_fingerprint = excluded.font_fingerprint,
                failure_code = excluded.failure_code,
                failure_message = excluded.failure_message,
                recovery_action = excluded.recovery_action,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE visual_baseline.status = 'failed'
               AND visual_baseline.project_id = excluded.project_id
               AND visual_baseline.manifest_digest = excluded.manifest_digest
               AND visual_baseline.package_commit_sha = excluded.package_commit_sha",
        )
        .bind(baseline.spec_version_id.as_str())
        .bind(baseline.project_id.as_str())
        .bind(&baseline.comparison_id)
        .bind(&baseline.manifest_digest)
        .bind(&baseline.package_commit_sha)
        .bind(baseline.status.as_str())
        .bind(&baseline.png_digest)
        .bind(&baseline.geometry_digest)
        .bind(&baseline.accessibility_digest)
        .bind(&baseline.environment_digest)
        .bind(&baseline.browser_version)
        .bind(&baseline.font_fingerprint)
        .bind(&baseline.failure_code)
        .bind(&baseline.failure_message)
        .bind(&baseline.recovery_action)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;

        if result.rows_affected() == 0 {
            let current =
                VisualBaselineStore::get(self, &baseline.spec_version_id, &baseline.comparison_id)
                    .await?
                    .ok_or_else(|| {
                        StoreError::CorruptRow("visual baseline write disappeared".into())
                    })?;
            if &current != baseline {
                return Err(StoreError::CorruptRow(
                    "ready visual baseline is immutable or retry provenance changed".into(),
                )
                .into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ports::VisualBaselineStore;

    fn failed() -> VisualBaseline {
        VisualBaseline {
            spec_version_id: SpecVersionId::new(test_fixtures::SPEC).unwrap(),
            project_id: test_fixtures::PROJECT.clone(),
            comparison_id: "home-default".into(),
            manifest_digest: "c".repeat(64),
            package_commit_sha: "1".repeat(40),
            status: VisualBaselineStatus::Failed,
            png_digest: None,
            geometry_digest: None,
            accessibility_digest: None,
            environment_digest: None,
            browser_version: None,
            font_fingerprint: None,
            failure_code: Some("timeout".into()),
            failure_message: Some("not ready".into()),
            recovery_action: Some("fix selector".into()),
        }
    }

    #[tokio::test]
    async fn failed_attempt_can_become_ready_but_ready_is_immutable() {
        let store = test_fixtures::store_with_approved_spec_without_baseline().await;
        let failed = failed();
        VisualBaselineStore::save(&store, &failed).await.unwrap();

        let mut ready = failed.clone();
        ready.status = VisualBaselineStatus::Ready;
        ready.png_digest = Some("d".repeat(64));
        ready.geometry_digest = Some("e".repeat(64));
        ready.accessibility_digest = Some("f".repeat(64));
        ready.environment_digest = Some("a".repeat(64));
        ready.browser_version = Some("Chrome/151".into());
        ready.font_fingerprint = Some("b".repeat(64));
        ready.failure_code = None;
        ready.failure_message = None;
        ready.recovery_action = None;
        VisualBaselineStore::save(&store, &ready).await.unwrap();
        VisualBaselineStore::save(&store, &ready).await.unwrap();

        let mut changed = ready;
        changed.png_digest = Some("0".repeat(64));
        assert!(VisualBaselineStore::save(&store, &changed).await.is_err());
    }
}
