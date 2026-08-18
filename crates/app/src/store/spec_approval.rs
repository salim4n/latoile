//! Atomic approval of one immutable architecture version. Domain transitions
//! are completed before this method is called; the transaction makes the
//! supersession, approval, project status, task binding and journal entry one
//! durable fact.

use super::{Store, StoreError};
use latoile_core::ports::PortResult;
use latoile_core::{Project, SpecProvenance, SpecVersion};

impl Store {
    pub async fn approve_spec_atomically(
        &self,
        spec: &SpecVersion,
        previous: Option<&SpecVersion>,
        project: &Project,
    ) -> PortResult<()> {
        let provenance = spec.provenance.as_ref().ok_or_else(|| {
            StoreError::CorruptRow("approved spec has no immutable provenance".into())
        })?;
        let mut transaction = self.pool().begin().await.map_err(StoreError::from)?;

        if let Some(previous) = previous {
            let updated = sqlx::query(
                "UPDATE spec_version SET status = 'superseded'
                 WHERE id = ? AND project_id = ? AND status = 'approved'",
            )
            .bind(previous.id.as_str())
            .bind(previous.project_id.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(StoreError::from)?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::CorruptRow(
                    "approved predecessor changed during spec approval".into(),
                )
                .into());
            }
        }

        let updated = update_verified_draft(&mut transaction, spec, provenance).await?;
        if updated != 1 {
            return Err(StoreError::CorruptRow(
                "draft provenance changed during immutable approval".into(),
            )
            .into());
        }

        sqlx::query(
            "UPDATE project SET status = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?",
        )
        .bind(project.status.as_str())
        .bind(project.id.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(StoreError::from)?;

        sqlx::query(
            "UPDATE task SET spec_version_id = ?,
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE project_id = ? AND spec_version_id IS NULL",
        )
        .bind(spec.id.as_str())
        .bind(spec.project_id.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(StoreError::from)?;

        sqlx::query("INSERT INTO event (project_id, kind, payload) VALUES (?, 'spec_approved', ?)")
            .bind(spec.project_id.as_str())
            .bind(
                serde_json::json!({
                    "spec_version_id": spec.id.as_str(),
                    "architecture_session_id": provenance.architecture_session_id.as_str(),
                    "skill_digest": provenance.skill_digest,
                    "package_digest": provenance.package_digest,
                    "manifest_digest": provenance.manifest_digest,
                    "package_commit_sha": provenance.package_commit_sha,
                    "package_tree_sha": provenance.package_tree_sha,
                })
                .to_string(),
            )
            .execute(&mut *transaction)
            .await
            .map_err(StoreError::from)?;

        transaction.commit().await.map_err(StoreError::from)?;
        Ok(())
    }
}

async fn update_verified_draft(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    spec: &SpecVersion,
    provenance: &SpecProvenance,
) -> Result<u64, StoreError> {
    let result = sqlx::query(
        "UPDATE spec_version SET status = 'approved'
         WHERE id = ? AND project_id = ? AND status = 'draft' AND design_dir = ?
           AND architecture_session_id = ? AND skill_name = ? AND skill_digest = ?
           AND operating_mode = ? AND package_digest = ? AND manifest_digest = ?
           AND package_commit_sha = ? AND package_tree_sha = ?",
    )
    .bind(spec.id.as_str())
    .bind(spec.project_id.as_str())
    .bind(&spec.design_dir)
    .bind(provenance.architecture_session_id.as_str())
    .bind(&provenance.skill_name)
    .bind(&provenance.skill_digest)
    .bind(provenance.operating_mode.as_str())
    .bind(&provenance.package_digest)
    .bind(&provenance.manifest_digest)
    .bind(&provenance.package_commit_sha)
    .bind(&provenance.package_tree_sha)
    .execute(&mut **transaction)
    .await?;
    Ok(result.rows_affected())
}
