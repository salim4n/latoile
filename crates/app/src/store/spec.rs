//! `spec_version` table — one approved spec per project, enforced by the
//! `one_approved_spec_per_project` partial index on top of the state machine.

use super::{Store, StoreError, unknown_variant};
use latoile_core::ports::{PortResult, SpecStore};
use latoile_core::{
    ArchitectureOperatingMode, ArchitectureSessionId, ProjectId, RunId, SpecProvenance, SpecStatus,
    SpecVersion, SpecVersionId,
};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

fn parse_status(raw: &str) -> Result<SpecStatus, StoreError> {
    Ok(match raw {
        "draft" => SpecStatus::Draft,
        "approved" => SpecStatus::Approved,
        "superseded" => SpecStatus::Superseded,
        other => return Err(unknown_variant("spec status", other)),
    })
}

fn row_to_spec(row: &SqliteRow) -> Result<SpecVersion, StoreError> {
    let architect = row.try_get::<Option<String>, _>("architect_run_id")?;
    let architecture_session = row.try_get::<Option<String>, _>("architecture_session_id")?;
    let provenance = architecture_session
        .map(|architecture_session_id| {
            let mode = match row.try_get::<String, _>("operating_mode")?.as_str() {
                "greenfield" => ArchitectureOperatingMode::Greenfield,
                "reverse_engineering" => ArchitectureOperatingMode::ReverseEngineering,
                other => return Err(unknown_variant("spec operating mode", other)),
            };
            Ok(SpecProvenance {
                architecture_session_id: ArchitectureSessionId::new(architecture_session_id)?,
                skill_name: row.try_get("skill_name")?,
                skill_digest: row.try_get("skill_digest")?,
                operating_mode: mode,
                package_digest: row.try_get("package_digest")?,
                manifest_digest: row
                    .try_get::<Option<String>, _>("manifest_digest")?
                    .unwrap_or_default(),
                package_commit_sha: row.try_get("package_commit_sha")?,
                package_tree_sha: row.try_get("package_tree_sha")?,
            })
        })
        .transpose()?;
    Ok(SpecVersion {
        id: SpecVersionId::new(row.try_get::<String, _>("id")?)?,
        project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)?,
        version: u32::try_from(row.try_get::<i64, _>("version")?)
            .map_err(|_| StoreError::CorruptRow("negative spec version".into()))?,
        status: parse_status(&row.try_get::<String, _>("status")?)?,
        design_dir: row.try_get("design_dir")?,
        architect_run_id: architect.map(RunId::new).transpose()?,
        provenance,
    })
}

const COLUMNS: &str = "id, project_id, version, status, design_dir, architect_run_id, architecture_session_id, \
     skill_name, skill_digest, operating_mode, package_digest, manifest_digest, package_commit_sha, package_tree_sha";

impl SpecStore for Store {
    async fn approved_for_project(&self, project: &ProjectId) -> PortResult<Option<SpecVersion>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM spec_version WHERE project_id = ? AND status = 'approved'"
        ))
        .bind(project.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.map(|r| row_to_spec(&r)).transpose().map_err(Into::into)
    }

    async fn save(&self, spec: &SpecVersion) -> PortResult<()> {
        let provenance = spec.provenance.as_ref();
        sqlx::query(
            "INSERT INTO spec_version
               (id, project_id, version, status, design_dir, architect_run_id,
                architecture_session_id, skill_name, skill_digest, operating_mode,
                package_digest, manifest_digest, package_commit_sha, package_tree_sha)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               status = excluded.status,
               design_dir = excluded.design_dir,
               architect_run_id = excluded.architect_run_id,
               architecture_session_id = excluded.architecture_session_id,
               skill_name = excluded.skill_name,
               skill_digest = excluded.skill_digest,
               operating_mode = excluded.operating_mode,
               package_digest = excluded.package_digest,
               manifest_digest = excluded.manifest_digest,
               package_commit_sha = excluded.package_commit_sha,
               package_tree_sha = excluded.package_tree_sha",
        )
        .bind(spec.id.as_str())
        .bind(spec.project_id.as_str())
        .bind(i64::from(spec.version))
        .bind(spec.status.as_str())
        .bind(&spec.design_dir)
        .bind(spec.architect_run_id.as_ref().map(|r| r.as_str()))
        .bind(provenance.map(|value| value.architecture_session_id.as_str()))
        .bind(provenance.map(|value| value.skill_name.as_str()))
        .bind(provenance.map(|value| value.skill_digest.as_str()))
        .bind(provenance.map(|value| value.operating_mode.as_str()))
        .bind(provenance.map(|value| value.package_digest.as_str()))
        .bind(provenance.map(|value| value.manifest_digest.as_str()))
        .bind(provenance.map(|value| value.package_commit_sha.as_str()))
        .bind(provenance.map(|value| value.package_tree_sha.as_str()))
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }
}

/// Reads the port does not cover: the spec list and detail routes need them.
/// They live here so SQL stays in the store (guardian #3); if the port grows
/// a `get`/`list`, these become the impl.
impl Store {
    /// All versions of a project's spec, newest first.
    pub async fn specs_for_project(
        &self,
        project: &latoile_core::ids::ProjectId,
    ) -> PortResult<Vec<SpecVersion>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM spec_version WHERE project_id = ? \
             ORDER BY version DESC"
        ))
        .bind(project.as_str())
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_spec)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// One version by id — the approve route fetches before it decides.
    pub async fn spec_by_id(
        &self,
        id: &latoile_core::ids::SpecVersionId,
    ) -> PortResult<Option<SpecVersion>> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM spec_version WHERE id = ?"))
            .bind(id.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(StoreError::from)?;
        row.map(|r| row_to_spec(&r)).transpose().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use latoile_core::Project;
    use latoile_core::ports::ProjectStore;

    async fn store_with_project() -> (Store, ProjectId) {
        let s = Store::open_ephemeral().await.unwrap();
        let p = Project::new(
            ProjectId::new("p1").unwrap(),
            "Mon App",
            "mon-app",
            "salim4n/mon-app",
            "work",
            "/srv/latoile/mon-app",
            "pnpm dev",
        )
        .unwrap();
        ProjectStore::save(&s, &p).await.unwrap();
        (s, p.id)
    }

    fn spec(id: &str, project: &ProjectId, version: u32) -> SpecVersion {
        SpecVersion::new(
            SpecVersionId::new(id).unwrap(),
            project.clone(),
            version,
            "design/",
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn round_trip() {
        let (s, project) = store_with_project().await;
        crate::store::test_fixtures::seed_test_architecture_session(&s).await;
        let mut v = spec("s1", &project, 1);
        crate::store::test_fixtures::attach_test_provenance(&mut v);
        let verification = crate::store::test_fixtures::test_verification(&v);
        v.approve(&verification).unwrap();
        SpecStore::save(&s, &v).await.unwrap();

        let back = s.approved_for_project(&project).await.unwrap().unwrap();
        assert_eq!(back, v);
    }

    #[tokio::test]
    async fn a_draft_is_not_the_approved_spec() {
        let (s, project) = store_with_project().await;
        crate::store::test_fixtures::seed_test_architecture_session(&s).await;
        SpecStore::save(&s, &spec("s1", &project, 1)).await.unwrap();
        assert!(s.approved_for_project(&project).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_second_approved_spec_is_refused_by_the_index() {
        let (s, project) = store_with_project().await;
        crate::store::test_fixtures::seed_test_architecture_session(&s).await;
        let mut a = spec("s1", &project, 1);
        crate::store::test_fixtures::attach_test_provenance(&mut a);
        let a_verification = crate::store::test_fixtures::test_verification(&a);
        a.approve(&a_verification).unwrap();
        SpecStore::save(&s, &a).await.unwrap();

        let mut b = spec("s2", &project, 2);
        crate::store::test_fixtures::attach_test_provenance(&mut b);
        let b_verification = crate::store::test_fixtures::test_verification(&b);
        b.approve(&b_verification).unwrap();
        // The application layer is expected to supersede `a` first; if it
        // forgets, the partial unique index is the backstop.
        assert!(SpecStore::save(&s, &b).await.is_err());

        a.supersede().unwrap();
        SpecStore::save(&s, &a).await.unwrap();
        SpecStore::save(&s, &b).await.unwrap();
        assert_eq!(
            s.approved_for_project(&project).await.unwrap().unwrap().id,
            b.id
        );
    }
}
