//! `preview` table — supervised dev servers. The
//! `one_active_preview_per_project` partial index backs invariant §3.2.6;
//! `error` and `stopped` previews keep their rows but free the slot.

use super::{unknown_variant, Store, StoreError};
use latoile_core::ports::{PortResult, PreviewStore};
use latoile_core::{Preview, PreviewId, PreviewStatus, ProjectId};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

fn parse_status(raw: &str) -> Result<PreviewStatus, StoreError> {
    Ok(match raw {
        "starting" => PreviewStatus::Starting,
        "ready" => PreviewStatus::Ready,
        "stale" => PreviewStatus::Stale,
        "error" => PreviewStatus::Error,
        "stopped" => PreviewStatus::Stopped,
        other => return Err(unknown_variant("preview status", other)),
    })
}

fn row_to_preview(row: &SqliteRow) -> Result<Preview, StoreError> {
    let port = u16::try_from(row.try_get::<i64, _>("port")?)
        .map_err(|_| StoreError::CorruptRow("port out of range".into()))?;
    let pid = row
        .try_get::<Option<i64>, _>("pid")?
        .map(u32::try_from)
        .transpose()
        .map_err(|_| StoreError::CorruptRow("pid out of range".into()))?;
    Ok(Preview {
        id: PreviewId::new(row.try_get::<String, _>("id")?)?,
        project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)?,
        port,
        status: parse_status(&row.try_get::<String, _>("status")?)?,
        branch: row.try_get("branch")?,
        pid,
    })
}

const COLUMNS: &str = "id, project_id, port, status, branch, pid";

impl PreviewStore for Store {
    async fn active_for_project(&self, project: &ProjectId) -> PortResult<Option<Preview>> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM preview
             WHERE project_id = ? AND status IN ('starting', 'ready', 'stale')"
        ))
        .bind(project.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.map(|r| row_to_preview(&r))
            .transpose()
            .map_err(Into::into)
    }

    async fn save(&self, preview: &Preview) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO preview (id, project_id, port, status, branch, pid)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               port = excluded.port,
               status = excluded.status,
               branch = excluded.branch,
               pid = excluded.pid",
        )
        .bind(preview.id.as_str())
        .bind(preview.project_id.as_str())
        .bind(i64::from(preview.port))
        .bind(preview.status.as_str())
        .bind(&preview.branch)
        .bind(preview.pid.map(i64::from))
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }
}

impl Store {
    /// Every preview whose database state claims a supervised process still
    /// exists. Startup and the health loop reconcile this set with the
    /// process-local supervisor registry.
    pub async fn active_previews(&self) -> PortResult<Vec<Preview>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM preview
             WHERE status IN ('starting', 'ready', 'stale')
             ORDER BY project_id, id"
        ))
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_preview)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;

    fn preview(id: &str) -> Preview {
        Preview::new(
            PreviewId::new(id).unwrap(),
            test_fixtures::PROJECT.clone(),
            4100,
            "work",
        )
    }

    #[tokio::test]
    async fn round_trip() {
        let s = test_fixtures::store_with_project().await;
        let mut p = preview("pr1");
        p.mark_ready(4242).unwrap();
        s.save(&p).await.unwrap();

        let back = s
            .active_for_project(&test_fixtures::PROJECT)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(back, p);
        assert_eq!(back.pid, Some(4242));
    }

    #[tokio::test]
    async fn the_second_active_preview_is_refused_by_the_index() {
        let s = test_fixtures::store_with_project().await;
        s.save(&preview("pr1")).await.unwrap();
        assert!(s.save(&preview("pr2")).await.is_err());

        // A stopped preview frees the project's slot.
        let mut p1 = preview("pr1");
        p1.stop().unwrap();
        s.save(&p1).await.unwrap();
        s.save(&preview("pr2")).await.unwrap();
    }

    #[tokio::test]
    async fn a_stopped_preview_is_no_longer_active() {
        let s = test_fixtures::store_with_project().await;
        let mut p = preview("pr1");
        p.mark_ready(1).unwrap();
        p.stop().unwrap();
        s.save(&p).await.unwrap();
        assert!(s
            .active_for_project(&test_fixtures::PROJECT)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn active_previews_returns_only_process_claims() {
        let s = test_fixtures::store_with_project().await;
        let mut active = preview("pr1");
        active.mark_ready(4242).unwrap();
        s.save(&active).await.unwrap();
        assert_eq!(s.active_previews().await.unwrap(), [active.clone()]);

        active.fail().unwrap();
        s.save(&active).await.unwrap();
        assert!(s.active_previews().await.unwrap().is_empty());
    }
}
