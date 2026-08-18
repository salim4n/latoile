//! `task` table — the board.

use super::{unknown_variant, Store, StoreError};
use latoile_core::ids::SpecVersionId;
use latoile_core::ports::{PortResult, TaskStore};
use latoile_core::{ProjectId, RoleId, Task, TaskId, TaskStatus};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

fn parse_status(raw: &str) -> Result<TaskStatus, StoreError> {
    Ok(match raw {
        "ready" => TaskStatus::Ready,
        "in_progress" => TaskStatus::InProgress,
        "review" => TaskStatus::Review,
        "changes_requested" => TaskStatus::ChangesRequested,
        "done" => TaskStatus::Done,
        other => return Err(unknown_variant("task status", other)),
    })
}

fn row_to_task(row: &SqliteRow) -> Result<Task, StoreError> {
    let spec = row.try_get::<Option<String>, _>("spec_version_id")?;
    Ok(Task {
        id: TaskId::new(row.try_get::<String, _>("id")?)?,
        project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)?,
        spec_version_id: spec.map(SpecVersionId::new).transpose()?,
        role_id: RoleId::new(row.try_get::<String, _>("role_id")?)?,
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        status: parse_status(&row.try_get::<String, _>("status")?)?,
        position: u32::try_from(row.try_get::<i64, _>("position")?)
            .map_err(|_| StoreError::CorruptRow("negative task position".into()))?,
    })
}

const COLUMNS: &str =
    "id, project_id, spec_version_id, role_id, title, description, status, position";

/// Board read model: the task plus its most recent run, when one exists.
/// Run identity is presentation context, so it stays out of the Task domain
/// entity and is joined only for the project workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTaskRow {
    pub task: Task,
    pub latest_run_id: Option<String>,
}

impl Store {
    pub async fn list_project_task_rows(
        &self,
        project: &ProjectId,
    ) -> PortResult<Vec<ProjectTaskRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS},
                    (SELECT run.id FROM run
                     WHERE run.task_id = task.id
                     ORDER BY run.started_at DESC, run.id DESC LIMIT 1) AS latest_run_id
             FROM task WHERE project_id = ?
             ORDER BY position, created_at, id"
        ))
        .bind(project.as_str())
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;

        rows.iter()
            .map(|row| {
                Ok(ProjectTaskRow {
                    task: row_to_task(row)?,
                    latest_run_id: row.try_get("latest_run_id")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()
            .map_err(Into::into)
    }
}

impl TaskStore for Store {
    async fn get(&self, id: &TaskId) -> PortResult<Option<Task>> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM task WHERE id = ?"))
            .bind(id.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(StoreError::from)?;
        row.map(|r| row_to_task(&r)).transpose().map_err(Into::into)
    }

    /// Board order: position, then creation for stability.
    async fn list_for_project(&self, project: &ProjectId) -> PortResult<Vec<Task>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM task WHERE project_id = ? ORDER BY position, created_at, id"
        ))
        .bind(project.as_str())
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_task)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn save(&self, task: &Task) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO task
               (id, project_id, spec_version_id, role_id, title, description,
                status, position)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               spec_version_id = excluded.spec_version_id,
               role_id = excluded.role_id,
               title = excluded.title,
               description = excluded.description,
               status = excluded.status,
               position = excluded.position,
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(task.id.as_str())
        .bind(task.project_id.as_str())
        .bind(task.spec_version_id.as_ref().map(|s| s.as_str()))
        .bind(task.role_id.as_str())
        .bind(&task.title)
        .bind(&task.description)
        .bind(task.status.as_str())
        .bind(i64::from(task.position))
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ids::SpecVersionId;

    fn task(id: &str, project: &ProjectId, position: u32) -> Task {
        Task::new(
            TaskId::new(id).unwrap(),
            project.clone(),
            RoleId::new("frontend").unwrap(),
            "Page de connexion",
            "Formulaire email + mot de passe",
            position,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn round_trip() {
        let s = test_fixtures::store_with_project().await;
        let t = task("t1", &test_fixtures::PROJECT, 0);
        s.save(&t).await.unwrap();

        let back = s.get(&t.id).await.unwrap().unwrap();
        assert_eq!(back, t);
    }

    #[tokio::test]
    async fn a_task_with_a_bound_spec_round_trips() {
        let s = test_fixtures::store_with_approved_spec().await;
        let mut t = task("t1", &test_fixtures::PROJECT, 0);
        t.bind_spec(SpecVersionId::new(test_fixtures::SPEC).unwrap());
        t.start().unwrap();
        s.save(&t).await.unwrap();

        let back = s.get(&t.id).await.unwrap().unwrap();
        assert_eq!(back, t);
        assert_eq!(back.status, TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn list_for_project_is_in_board_order() {
        let s = test_fixtures::store_with_project().await;
        s.save(&task("t2", &test_fixtures::PROJECT, 1))
            .await
            .unwrap();
        s.save(&task("t1", &test_fixtures::PROJECT, 0))
            .await
            .unwrap();

        let listed = s.list_for_project(&test_fixtures::PROJECT).await.unwrap();
        assert_eq!(listed[0].id.as_str(), "t1");
        assert_eq!(listed[1].id.as_str(), "t2");
    }

    #[tokio::test]
    async fn a_task_rejects_an_unknown_role() {
        let s = test_fixtures::store_with_project().await;
        let mut t = task("t1", &test_fixtures::PROJECT, 0);
        t.role_id = RoleId::new("ghost").unwrap();
        assert!(s.save(&t).await.is_err());
    }

    #[tokio::test]
    async fn project_rows_include_the_latest_run() {
        let (s, run_id) = test_fixtures::store_with_run().await;

        let rows = s
            .list_project_task_rows(&test_fixtures::PROJECT)
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task.id.as_str(), "t1");
        assert_eq!(rows[0].latest_run_id.as_deref(), Some(run_id.as_str()));
    }
}
