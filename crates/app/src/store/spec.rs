//! `spec_version` table — one approved spec per project, enforced by the
//! `one_approved_spec_per_project` partial index on top of the state machine.

use super::{unknown_variant, Store, StoreError};
use latoile_core::ports::{PortResult, SpecStore};
use latoile_core::{ProjectId, RunId, SpecStatus, SpecVersion, SpecVersionId};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

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
    Ok(SpecVersion {
        id: SpecVersionId::new(row.try_get::<String, _>("id")?)?,
        project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)?,
        version: u32::try_from(row.try_get::<i64, _>("version")?)
            .map_err(|_| StoreError::CorruptRow("negative spec version".into()))?,
        status: parse_status(&row.try_get::<String, _>("status")?)?,
        design_dir: row.try_get("design_dir")?,
        architect_run_id: architect.map(RunId::new).transpose()?,
    })
}

const COLUMNS: &str =
    "id, project_id, version, status, design_dir, architect_run_id";

impl SpecStore for Store {
    async fn approved_for_project(
        &self,
        project: &ProjectId,
    ) -> PortResult<Option<SpecVersion>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM spec_version WHERE project_id = ? AND status = 'approved'"
        ))
        .bind(project.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.map(|r| row_to_spec(&r))
            .transpose()
            .map_err(Into::into)
    }

    async fn save(&self, spec: &SpecVersion) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO spec_version
               (id, project_id, version, status, design_dir, architect_run_id)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               status = excluded.status,
               design_dir = excluded.design_dir,
               architect_run_id = excluded.architect_run_id",
        )
        .bind(spec.id.as_str())
        .bind(spec.project_id.as_str())
        .bind(i64::from(spec.version))
        .bind(spec.status.as_str())
        .bind(&spec.design_dir)
        .bind(spec.architect_run_id.as_ref().map(|r| r.as_str()))
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use latoile_core::ports::ProjectStore;
    use latoile_core::Project;

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
        let mut v = spec("s1", &project, 1);
        v.approve().unwrap();
        SpecStore::save(&s, &v).await.unwrap();

        let back = s.approved_for_project(&project).await.unwrap().unwrap();
        assert_eq!(back, v);
    }

    #[tokio::test]
    async fn a_draft_is_not_the_approved_spec() {
        let (s, project) = store_with_project().await;
        SpecStore::save(&s, &spec("s1", &project, 1)).await.unwrap();
        assert!(s.approved_for_project(&project).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_second_approved_spec_is_refused_by_the_index() {
        let (s, project) = store_with_project().await;
        let mut a = spec("s1", &project, 1);
        a.approve().unwrap();
        SpecStore::save(&s, &a).await.unwrap();

        let mut b = spec("s2", &project, 2);
        b.approve().unwrap();
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
