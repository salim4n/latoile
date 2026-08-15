//! `project` table — the aggregate everything else hangs off.

use super::{unknown_variant, Store, StoreError};
use latoile_core::ports::{PortResult, ProjectStore};
use latoile_core::{Project, ProjectId, ProjectStatus};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

fn parse_status(raw: &str) -> Result<ProjectStatus, StoreError> {
    Ok(match raw {
        "draft" => ProjectStatus::Draft,
        "specced" => ProjectStatus::Specced,
        "building" => ProjectStatus::Building,
        "live" => ProjectStatus::Live,
        other => return Err(unknown_variant("project status", other)),
    })
}

fn row_to_project(row: &SqliteRow) -> Result<Project, StoreError> {
    Ok(Project {
        id: ProjectId::new(row.try_get::<String, _>("id")?)?,
        name: row.try_get("name")?,
        slug: row.try_get("slug")?,
        github_repo: row.try_get("github_repo")?,
        default_branch: row.try_get("default_branch")?,
        work_branch: row.try_get("work_branch")?,
        local_path: row.try_get("local_path")?,
        status: parse_status(&row.try_get::<String, _>("status")?)?,
        dev_command: row.try_get("dev_command")?,
        deleted: row.try_get::<i64, _>("deleted")? != 0,
    })
}

const COLUMNS: &str = "id, name, slug, github_repo, default_branch, work_branch, \
                       local_path, status, dev_command, deleted";

impl ProjectStore for Store {
    async fn get(&self, id: &ProjectId) -> PortResult<Option<Project>> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM project WHERE id = ?"))
            .bind(id.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(StoreError::from)?;
        row.map(|r| row_to_project(&r))
            .transpose()
            .map_err(Into::into)
    }

    /// The board never shows soft-deleted projects.
    async fn list(&self) -> PortResult<Vec<Project>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM project WHERE deleted = 0 ORDER BY created_at, id"
        ))
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_project)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn save(&self, project: &Project) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO project
               (id, name, slug, github_repo, default_branch, work_branch,
                local_path, status, dev_command, deleted)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               slug = excluded.slug,
               github_repo = excluded.github_repo,
               default_branch = excluded.default_branch,
               work_branch = excluded.work_branch,
               local_path = excluded.local_path,
               status = excluded.status,
               dev_command = excluded.dev_command,
               deleted = excluded.deleted,
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(project.id.as_str())
        .bind(&project.name)
        .bind(&project.slug)
        .bind(&project.github_repo)
        .bind(&project.default_branch)
        .bind(&project.work_branch)
        .bind(&project.local_path)
        .bind(project.status.as_str())
        .bind(&project.dev_command)
        .bind(i64::from(project.deleted))
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> Store {
        Store::open_ephemeral().await.unwrap()
    }

    fn project(id: &str, slug: &str) -> Project {
        Project::new(
            ProjectId::new(id).unwrap(),
            "Mon App",
            slug,
            "salim4n/mon-app",
            "work",
            "/srv/latoile/mon-app",
            "pnpm dev --port $PORT",
        )
        .unwrap()
    }

    #[tokio::test]
    async fn round_trip() {
        let s = store().await;
        let p = project("p1", "mon-app");
        s.save(&p).await.unwrap();

        let back = ProjectStore::get(&s, &p.id).await.unwrap().unwrap();
        assert_eq!(back, p);
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown_ids() {
        let s = store().await;
        assert!(ProjectStore::get(&s, &ProjectId::new("nope").unwrap())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn save_is_an_upsert() {
        let s = store().await;
        let mut p = project("p1", "mon-app");
        s.save(&p).await.unwrap();

        p.mark_specced();
        p.mark_building();
        s.save(&p).await.unwrap();

        let back = ProjectStore::get(&s, &p.id).await.unwrap().unwrap();
        assert_eq!(back.status, ProjectStatus::Building);
    }

    #[tokio::test]
    async fn list_hides_soft_deleted_projects() {
        let s = store().await;
        let mut gone = project("p1", "gone");
        s.save(&gone).await.unwrap();
        s.save(&project("p2", "kept")).await.unwrap();

        gone.soft_delete();
        s.save(&gone).await.unwrap();

        let listed = s.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "kept");
        // …but a direct fetch still finds it (audit trail).
        assert!(ProjectStore::get(&s, &gone.id).await.unwrap().unwrap().deleted);
    }

    #[tokio::test]
    async fn slug_is_unique() {
        let s = store().await;
        s.save(&project("p1", "dup")).await.unwrap();
        assert!(s.save(&project("p2", "dup")).await.is_err());
    }
}
