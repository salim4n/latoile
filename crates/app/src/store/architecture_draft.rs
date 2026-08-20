//! Atomic persistence of a verified architecture package and its draft spec.
//! The filesystem commit is already immutable at this point; SQLite must
//! expose the session, draft and journal event together or not at all.

use super::{Store, StoreError};
use latoile_core::ports::PortResult;
use latoile_core::{ArchitectureSession, SpecVersion};

impl Store {
    pub async fn save_architecture_draft(
        &self,
        session: &ArchitectureSession,
        spec: &SpecVersion,
    ) -> PortResult<()> {
        let package = session.package.as_ref().ok_or_else(|| {
            StoreError::CorruptRow(
                "draft-ready architecture session has no package evidence".into(),
            )
        })?;
        let provenance = spec
            .provenance
            .as_ref()
            .ok_or_else(|| StoreError::CorruptRow("architecture draft has no provenance".into()))?;
        let changed_files = serde_json::to_string(&package.changed_files)
            .map_err(|error| StoreError::CorruptRow(error.to_string()))?;
        let mut transaction = self.pool().begin().await.map_err(StoreError::from)?;

        let updated = sqlx::query(
            "UPDATE architecture_session SET
               status = ?, phase = ?, acp_session_id = ?, skill_name = ?, skill_digest = ?,
               operating_mode = ?, requested_locale = ?, package_status = ?, package_design_dir = ?,
               package_base_sha = ?, package_head_sha = ?, package_tree_sha = ?,
               package_digest = ?, package_manifest_digest = ?, package_changed_files = ?, package_diff_stat = ?,
               failure_reason = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?",
        )
        .bind(session.status.as_str())
        .bind(session.phase.as_str())
        .bind(&session.acp_session_id)
        .bind(&session.skill_name)
        .bind(&session.skill_digest)
        .bind(session.operating_mode.map(|mode| mode.as_str()))
        .bind(&session.requested_locale)
        .bind(session.package_status.as_str())
        .bind(&package.design_dir)
        .bind(&package.base_sha)
        .bind(&package.head_sha)
        .bind(&package.tree_sha)
        .bind(&package.package_digest)
        .bind(&package.manifest_digest)
        .bind(changed_files)
        .bind(&package.diff_stat)
        .bind(&session.failure_reason)
        .bind(session.id.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(StoreError::from)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::CorruptRow(
                "architecture session disappeared before draft persistence".into(),
            )
            .into());
        }

        sqlx::query(
            "INSERT INTO spec_version
               (id, project_id, version, status, design_dir, architect_run_id,
                architecture_session_id, skill_name, skill_digest, operating_mode,
                package_digest, manifest_digest, package_commit_sha, package_tree_sha)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(spec.id.as_str())
        .bind(spec.project_id.as_str())
        .bind(i64::from(spec.version))
        .bind(spec.status.as_str())
        .bind(&spec.design_dir)
        .bind(spec.architect_run_id.as_ref().map(|run| run.as_str()))
        .bind(provenance.architecture_session_id.as_str())
        .bind(&provenance.skill_name)
        .bind(&provenance.skill_digest)
        .bind(provenance.operating_mode.as_str())
        .bind(&provenance.package_digest)
        .bind(&provenance.manifest_digest)
        .bind(&provenance.package_commit_sha)
        .bind(&provenance.package_tree_sha)
        .execute(&mut *transaction)
        .await
        .map_err(StoreError::from)?;

        sqlx::query(
            "INSERT INTO event (project_id, kind, payload) VALUES (?, 'spec_version_created', ?)",
        )
        .bind(spec.project_id.as_str())
        .bind(format!("{{\"spec_version_id\":\"{}\"}}", spec.id.as_str()))
        .execute(&mut *transaction)
        .await
        .map_err(StoreError::from)?;

        transaction.commit().await.map_err(StoreError::from)?;
        Ok(())
    }
}
