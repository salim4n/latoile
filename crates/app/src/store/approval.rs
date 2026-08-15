//! `approval` table — the human's decision points.

use super::{unknown_variant, Store, StoreError};
use latoile_core::ports::{ApprovalStore, PortResult};
use latoile_core::{Approval, ApprovalId, ApprovalKind, ApprovalStatus, RunId};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

fn parse_kind(raw: &str) -> Result<ApprovalKind, StoreError> {
    Ok(match raw {
        "spec" => ApprovalKind::Spec,
        "review" => ApprovalKind::Review,
        "permission" => ApprovalKind::Permission,
        other => return Err(unknown_variant("approval kind", other)),
    })
}

fn parse_status(raw: &str) -> Result<ApprovalStatus, StoreError> {
    Ok(match raw {
        "pending" => ApprovalStatus::Pending,
        "granted" => ApprovalStatus::Granted,
        "rejected" => ApprovalStatus::Rejected,
        other => return Err(unknown_variant("approval status", other)),
    })
}

fn row_to_approval(row: &SqliteRow) -> Result<Approval, StoreError> {
    Ok(Approval {
        id: ApprovalId::new(row.try_get::<String, _>("id")?)?,
        run_id: RunId::new(row.try_get::<String, _>("run_id")?)?,
        kind: parse_kind(&row.try_get::<String, _>("kind")?)?,
        status: parse_status(&row.try_get::<String, _>("status")?)?,
        payload: row.try_get("payload")?,
    })
}

const COLUMNS: &str = "id, run_id, kind, status, payload";

impl ApprovalStore for Store {
    /// The inbox: everything still waiting for the owner, oldest first.
    async fn list_pending(&self) -> PortResult<Vec<Approval>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM approval WHERE status = 'pending' \
             ORDER BY created_at, id"
        ))
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_approval)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn save(&self, approval: &Approval) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO approval (id, run_id, kind, status, payload)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               status = excluded.status,
               payload = excluded.payload,
               decided_at = CASE
                   WHEN excluded.status IN ('granted', 'rejected')
                   THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                   ELSE NULL END",
        )
        .bind(approval.id.as_str())
        .bind(approval.run_id.as_str())
        .bind(approval.kind.as_str())
        .bind(approval.status.as_str())
        .bind(&approval.payload)
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

    fn approval(id: &str, run: &str, kind: ApprovalKind) -> Approval {
        Approval::new(
            ApprovalId::new(id).unwrap(),
            RunId::new(run).unwrap(),
            kind,
            "{}".into(),
        )
    }

    #[tokio::test]
    async fn pending_approvals_are_the_inbox() {
        let (s, run) = test_fixtures::store_with_run().await;

        let mut granted = approval("a1", run.as_str(), ApprovalKind::Review);
        granted.grant().unwrap();
        s.save(&granted).await.unwrap();
        s.save(&approval("a2", run.as_str(), ApprovalKind::Permission))
            .await
            .unwrap();

        let pending = s.list_pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id.as_str(), "a2");
        assert_eq!(pending[0].status, ApprovalStatus::Pending);
    }

    #[tokio::test]
    async fn decisions_round_trip() {
        let (s, run) = test_fixtures::store_with_run().await;
        let mut a = approval("a1", run.as_str(), ApprovalKind::Review);
        s.save(&a).await.unwrap();

        a.reject().unwrap();
        s.save(&a).await.unwrap();
        assert!(s.list_pending().await.unwrap().is_empty());
    }
}
