//! `run` table — agent executions. The `one_active_run_per_task` partial
//! index backs the domain's single-active-run invariant (§3.2.1).

use super::{unknown_variant, Store, StoreError};
use latoile_core::ports::{PortResult, RunStore};
use latoile_core::{RoleId, Run, RunId, RunStatus, TaskId, TriggeredBy};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

fn parse_status(raw: &str) -> Result<RunStatus, StoreError> {
    Ok(match raw {
        "starting" => RunStatus::Starting,
        "running" => RunStatus::Running,
        "blocked" => RunStatus::Blocked,
        "finished" => RunStatus::Finished,
        "error" => RunStatus::Error,
        "cancelled" => RunStatus::Cancelled,
        other => return Err(unknown_variant("run status", other)),
    })
}

fn parse_trigger(raw: &str) -> Result<TriggeredBy, StoreError> {
    Ok(match raw {
        "user" => TriggeredBy::User,
        "manager" => TriggeredBy::Manager,
        other => return Err(unknown_variant("run trigger", other)),
    })
}

fn row_to_run(row: &SqliteRow) -> Result<Run, StoreError> {
    Ok(Run {
        id: RunId::new(row.try_get::<String, _>("id")?)?,
        task_id: TaskId::new(row.try_get::<String, _>("task_id")?)?,
        role_id: RoleId::new(row.try_get::<String, _>("role_id")?)?,
        triggered_by: parse_trigger(&row.try_get::<String, _>("triggered_by")?)?,
        acp_session_id: row.try_get("acp_session_id")?,
        status: parse_status(&row.try_get::<String, _>("status")?)?,
        summary: row.try_get("summary")?,
        base_sha: row.try_get("base_sha")?,
        head_sha: row.try_get("head_sha")?,
        artifacts: row.try_get("artifacts")?,
    })
}

const COLUMNS: &str = "id, task_id, role_id, triggered_by, acp_session_id, status, summary, \
                       base_sha, head_sha, artifacts";

impl RunStore for Store {
    async fn get(&self, id: &RunId) -> PortResult<Option<Run>> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM run WHERE id = ?"))
            .bind(id.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(StoreError::from)?;
        row.map(|r| row_to_run(&r)).transpose().map_err(Into::into)
    }

    async fn list_for_task(&self, task: &TaskId) -> PortResult<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM run WHERE task_id = ? ORDER BY started_at, id"
        ))
        .bind(task.as_str())
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_run)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn active_for_task(&self, task: &TaskId) -> PortResult<Option<Run>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM run
             WHERE task_id = ? AND status IN ('starting', 'running', 'blocked')"
        ))
        .bind(task.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.map(|r| row_to_run(&r)).transpose().map_err(Into::into)
    }

    async fn save(&self, run: &Run) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO run (id, task_id, role_id, triggered_by, acp_session_id, status, summary,
                              base_sha, head_sha, artifacts)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               acp_session_id = excluded.acp_session_id,
               status = excluded.status,
               summary = excluded.summary,
               base_sha = excluded.base_sha,
               head_sha = excluded.head_sha,
               artifacts = excluded.artifacts,
               ended_at = CASE
                   WHEN excluded.status IN ('finished', 'error', 'cancelled')
                   THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                   ELSE NULL END",
        )
        .bind(run.id.as_str())
        .bind(run.task_id.as_str())
        .bind(run.role_id.as_str())
        .bind(match run.triggered_by {
            TriggeredBy::User => "user",
            TriggeredBy::Manager => "manager",
        })
        .bind(&run.acp_session_id)
        .bind(run.status.as_str())
        .bind(&run.summary)
        .bind(&run.base_sha)
        .bind(&run.head_sha)
        .bind(&run.artifacts)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }
}

impl Store {
    /// Every run still holding a task's slot — the supervision loop's poll
    /// set. Not a port method: nothing outside the driver needs it.
    pub async fn active_runs(&self) -> PortResult<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM run WHERE status IN ('starting', 'running', 'blocked')"
        ))
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_run)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store_with_task() -> (Store, TaskId) {
        let (s, task) = crate::store::test_fixtures::store_with_task().await;
        (s, task)
    }

    fn run(id: &str, task: &TaskId) -> Run {
        Run::new(
            RunId::new(id).unwrap(),
            task.clone(),
            RoleId::new("backend").unwrap(),
            TriggeredBy::Manager,
        )
    }

    #[tokio::test]
    async fn round_trip() {
        let (s, task) = store_with_task().await;
        let mut r = run("r1", &task);
        r.begin().unwrap();
        r.acp_session_id = Some("acp-1".into());
        r.finish("implemented and tested").unwrap();
        r.attach_evidence(
            Some("1111111".into()),
            Some("2222222".into()),
            r#"{"commits":[{"sha":"2222222","subject":"feat: ship"}]}"#.into(),
        )
        .unwrap();
        s.save(&r).await.unwrap();

        let back = s.get(&r.id).await.unwrap().unwrap();
        assert_eq!(back, r);
    }

    #[tokio::test]
    async fn active_for_task_sees_only_active_statuses() {
        let (s, task) = store_with_task().await;
        let mut finished = run("r1", &task);
        finished.begin().unwrap();
        finished.finish("done").unwrap();
        s.save(&finished).await.unwrap();
        assert!(s.active_for_task(&task).await.unwrap().is_none());

        let mut active = run("r2", &task);
        active.begin().unwrap();
        active.block().unwrap();
        s.save(&active).await.unwrap();
        assert_eq!(
            s.active_for_task(&task).await.unwrap().unwrap().status,
            RunStatus::Blocked
        );
    }

    #[tokio::test]
    async fn the_second_active_run_is_refused_by_the_index() {
        let (s, task) = store_with_task().await;
        s.save(&run("r1", &task)).await.unwrap();
        assert!(s.save(&run("r2", &task)).await.is_err());

        // Once the first run reaches a terminal state, the slot frees up.
        let mut r1 = run("r1", &task);
        r1.fail().unwrap();
        s.save(&r1).await.unwrap();
        s.save(&run("r2", &task)).await.unwrap();
        assert_eq!(s.list_for_task(&task).await.unwrap().len(), 2);
    }
}
