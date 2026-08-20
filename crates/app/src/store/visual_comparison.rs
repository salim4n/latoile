//! Immutable trusted comparisons. Invalid capture attempts are retryable for
//! the exact same run/spec/baseline tuple; complete evidence never changes.

use super::{Store, StoreError, unknown_variant};
use latoile_core::ids::{ProjectId, RunId, SpecVersionId, VisualComparisonId};
use latoile_core::ports::{PortResult, VisualComparisonStore};
use latoile_core::{VisualComparison, VisualComparisonStatus};
use sqlx::Row;

fn non_negative<T>(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<T, StoreError>
where
    T: TryFrom<i64>,
{
    let value = row.try_get::<i64, _>(column)?;
    T::try_from(value).map_err(|_| StoreError::CorruptRow(format!("invalid {column}: {value}")))
}

fn row_to_comparison(row: &sqlx::sqlite::SqliteRow) -> Result<VisualComparison, StoreError> {
    let status = match row.try_get::<String, _>("status")?.as_str() {
        "invalid" => VisualComparisonStatus::Invalid,
        "blocking" => VisualComparisonStatus::Blocking,
        "reservation" => VisualComparisonStatus::Reservation,
        "passed" => VisualComparisonStatus::Passed,
        raw => return Err(unknown_variant("visual comparison status", raw)),
    };
    Ok(VisualComparison {
        id: VisualComparisonId::new(row.try_get::<String, _>("id")?)?,
        spec_version_id: SpecVersionId::new(row.try_get::<String, _>("spec_version_id")?)?,
        project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)?,
        run_id: RunId::new(row.try_get::<String, _>("run_id")?)?,
        comparison_id: row.try_get("comparison_id")?,
        manifest_digest: row.try_get("manifest_digest")?,
        package_commit_sha: row.try_get("package_commit_sha")?,
        baseline_png_digest: row.try_get("baseline_png_digest")?,
        status,
        changed_pixels: non_negative(row, "changed_pixels")?,
        total_pixels: non_negative(row, "total_pixels")?,
        pixel_ratio_micros: non_negative(row, "pixel_ratio_micros")?,
        max_geometry_delta_milli: non_negative(row, "max_geometry_delta_milli")?,
        accessibility_changes: non_negative(row, "accessibility_changes")?,
        render_png_digest: row.try_get("render_png_digest")?,
        pixel_diff_digest: row.try_get("pixel_diff_digest")?,
        heatmap_png_digest: row.try_get("heatmap_png_digest")?,
        geometry_diff_digest: row.try_get("geometry_diff_digest")?,
        accessibility_diff_digest: row.try_get("accessibility_diff_digest")?,
        environment_digest: row.try_get("environment_digest")?,
        browser_version: row.try_get("browser_version")?,
        font_fingerprint: row.try_get("font_fingerprint")?,
        failure_code: row.try_get("failure_code")?,
        failure_message: row.try_get("failure_message")?,
        recovery_action: row.try_get("recovery_action")?,
    })
}

fn sqlite_integer<T>(value: T, column: &str) -> Result<i64, StoreError>
where
    i64: TryFrom<T>,
    T: Copy + std::fmt::Display,
{
    i64::try_from(value)
        .map_err(|_| StoreError::CorruptRow(format!("{column} does not fit SQLite: {value}")))
}

impl VisualComparisonStore for Store {
    async fn get(&self, id: &VisualComparisonId) -> PortResult<Option<VisualComparison>> {
        let row = sqlx::query("SELECT * FROM visual_comparison WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(StoreError::from)?;
        row.as_ref()
            .map(row_to_comparison)
            .transpose()
            .map_err(Into::into)
    }

    async fn list_for_run(&self, run: &RunId) -> PortResult<Vec<VisualComparison>> {
        let rows =
            sqlx::query("SELECT * FROM visual_comparison WHERE run_id = ? ORDER BY comparison_id")
                .bind(run.as_str())
                .fetch_all(self.pool())
                .await
                .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_comparison)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn save(&self, comparison: &VisualComparison) -> PortResult<()> {
        let result = sqlx::query(
            "INSERT INTO visual_comparison (
                id, spec_version_id, project_id, run_id, comparison_id,
                manifest_digest, package_commit_sha, baseline_png_digest, status,
                changed_pixels, total_pixels, pixel_ratio_micros,
                max_geometry_delta_milli, accessibility_changes,
                render_png_digest, pixel_diff_digest, heatmap_png_digest,
                geometry_diff_digest, accessibility_diff_digest, environment_digest,
                browser_version, font_fingerprint, failure_code, failure_message,
                recovery_action
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                changed_pixels = excluded.changed_pixels,
                total_pixels = excluded.total_pixels,
                pixel_ratio_micros = excluded.pixel_ratio_micros,
                max_geometry_delta_milli = excluded.max_geometry_delta_milli,
                accessibility_changes = excluded.accessibility_changes,
                render_png_digest = excluded.render_png_digest,
                pixel_diff_digest = excluded.pixel_diff_digest,
                heatmap_png_digest = excluded.heatmap_png_digest,
                geometry_diff_digest = excluded.geometry_diff_digest,
                accessibility_diff_digest = excluded.accessibility_diff_digest,
                environment_digest = excluded.environment_digest,
                browser_version = excluded.browser_version,
                font_fingerprint = excluded.font_fingerprint,
                failure_code = excluded.failure_code,
                failure_message = excluded.failure_message,
                recovery_action = excluded.recovery_action,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE visual_comparison.status = 'invalid'
               AND visual_comparison.spec_version_id = excluded.spec_version_id
               AND visual_comparison.project_id = excluded.project_id
               AND visual_comparison.run_id = excluded.run_id
               AND visual_comparison.comparison_id = excluded.comparison_id
               AND visual_comparison.manifest_digest = excluded.manifest_digest
               AND visual_comparison.package_commit_sha = excluded.package_commit_sha
               AND visual_comparison.baseline_png_digest = excluded.baseline_png_digest",
        )
        .bind(comparison.id.as_str())
        .bind(comparison.spec_version_id.as_str())
        .bind(comparison.project_id.as_str())
        .bind(comparison.run_id.as_str())
        .bind(&comparison.comparison_id)
        .bind(&comparison.manifest_digest)
        .bind(&comparison.package_commit_sha)
        .bind(&comparison.baseline_png_digest)
        .bind(comparison.status.as_str())
        .bind(sqlite_integer(comparison.changed_pixels, "changed_pixels")?)
        .bind(sqlite_integer(comparison.total_pixels, "total_pixels")?)
        .bind(i64::from(comparison.pixel_ratio_micros))
        .bind(i64::from(comparison.max_geometry_delta_milli))
        .bind(i64::from(comparison.accessibility_changes))
        .bind(&comparison.render_png_digest)
        .bind(&comparison.pixel_diff_digest)
        .bind(&comparison.heatmap_png_digest)
        .bind(&comparison.geometry_diff_digest)
        .bind(&comparison.accessibility_diff_digest)
        .bind(&comparison.environment_digest)
        .bind(&comparison.browser_version)
        .bind(&comparison.font_fingerprint)
        .bind(&comparison.failure_code)
        .bind(&comparison.failure_message)
        .bind(&comparison.recovery_action)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;

        if result.rows_affected() == 0 {
            let current = VisualComparisonStore::get(self, &comparison.id)
                .await?
                .ok_or_else(|| {
                    StoreError::CorruptRow("visual comparison write disappeared".into())
                })?;
            if &current != comparison {
                return Err(StoreError::CorruptRow(
                    "complete visual comparison is immutable or retry provenance changed".into(),
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
    use latoile_core::ports::VisualComparisonStore;

    fn invalid(run: &RunId) -> VisualComparison {
        VisualComparison {
            id: VisualComparisonId::new(format!("vc-{}-home", run.as_str())).unwrap(),
            spec_version_id: SpecVersionId::new(test_fixtures::SPEC).unwrap(),
            project_id: test_fixtures::PROJECT.clone(),
            run_id: run.clone(),
            comparison_id: "home-default".into(),
            manifest_digest: "c".repeat(64),
            package_commit_sha: "1".repeat(40),
            baseline_png_digest: "d".repeat(64),
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
            failure_code: Some("timeout".into()),
            failure_message: Some("not ready".into()),
            recovery_action: Some("fix route".into()),
        }
    }

    #[tokio::test]
    async fn invalid_attempt_can_become_complete_but_complete_is_immutable() {
        let store = test_fixtures::store_with_finished_frontend_run().await;
        let run = RunId::new(test_fixtures::FINISHED_RUN).unwrap();
        let invalid = invalid(&run);
        VisualComparisonStore::save(&store, &invalid).await.unwrap();

        let mut passed = invalid.clone();
        passed.status = VisualComparisonStatus::Passed;
        passed.total_pixels = 100;
        passed.render_png_digest = Some("1".repeat(64));
        passed.pixel_diff_digest = Some("2".repeat(64));
        passed.heatmap_png_digest = Some("3".repeat(64));
        passed.geometry_diff_digest = Some("4".repeat(64));
        passed.accessibility_diff_digest = Some("5".repeat(64));
        passed.environment_digest = Some("6".repeat(64));
        passed.browser_version = Some("Chrome/151".into());
        passed.font_fingerprint = Some("7".repeat(64));
        passed.failure_code = None;
        passed.failure_message = None;
        passed.recovery_action = None;
        VisualComparisonStore::save(&store, &passed).await.unwrap();
        VisualComparisonStore::save(&store, &passed).await.unwrap();

        let mut changed = passed;
        changed.status = VisualComparisonStatus::Blocking;
        assert!(VisualComparisonStore::save(&store, &changed).await.is_err());
    }
}
