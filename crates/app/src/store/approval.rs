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
        decision_comment: row.try_get("decision_comment")?,
        corrective_run_id: row
            .try_get::<Option<String>, _>("corrective_run_id")?
            .map(RunId::new)
            .transpose()?,
    })
}

const COLUMNS: &str = "id, run_id, kind, status, payload, decision_comment, corrective_run_id";

/// Query-side projection for the owner Inbox. Audit/context columns stay out
/// of the domain entity, while the UI receives enough real data to explain a
/// pending decision without issuing one request per run/task/project.
pub struct InboxApprovalRow {
    pub approval: Approval,
    pub project_id: String,
    pub project_name: String,
    pub task_title: String,
    pub role_id: String,
    pub created_at: String,
    pub decided_at: Option<String>,
}

impl Store {
    pub async fn pending_permissions_for_run(&self, run_id: &RunId) -> PortResult<Vec<Approval>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM approval
             WHERE run_id = ? AND kind = 'permission' AND status = 'pending'
             ORDER BY created_at, id"
        ))
        .bind(run_id.as_str())
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(row_to_approval)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub async fn list_pending_for_inbox(&self) -> PortResult<Vec<InboxApprovalRow>> {
        let rows = sqlx::query(
            "SELECT a.id, a.run_id, a.kind, a.status, a.payload,
                    a.decision_comment, a.corrective_run_id,
                    t.project_id, p.name AS project_name, t.title AS task_title,
                    r.role_id, a.created_at, a.decided_at
             FROM approval a
             JOIN run r ON r.id = a.run_id
             JOIN task t ON t.id = r.task_id
             JOIN project p ON p.id = t.project_id
             WHERE a.status = 'pending' AND p.deleted = 0
             ORDER BY a.created_at, a.id",
        )
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;

        rows.iter()
            .map(|row| {
                Ok(InboxApprovalRow {
                    approval: row_to_approval(row)?,
                    project_id: row.try_get("project_id")?,
                    project_name: row.try_get("project_name")?,
                    task_title: row.try_get("task_title")?,
                    role_id: row.try_get("role_id")?,
                    created_at: row.try_get("created_at")?,
                    decided_at: row.try_get("decided_at")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()
            .map_err(Into::into)
    }

    /// One approval with its project/task context, including terminal
    /// decisions. The Review screen uses it as an audit view after reload.
    pub async fn approval_detail(&self, id: &ApprovalId) -> PortResult<Option<InboxApprovalRow>> {
        let row = sqlx::query(
            "SELECT a.id, a.run_id, a.kind, a.status, a.payload,
                    a.decision_comment, a.corrective_run_id,
                    t.project_id, p.name AS project_name, t.title AS task_title,
                    r.role_id, a.created_at, a.decided_at
             FROM approval a
             JOIN run r ON r.id = a.run_id
             JOIN task t ON t.id = r.task_id
             JOIN project p ON p.id = t.project_id
             WHERE a.id = ? AND p.deleted = 0",
        )
        .bind(id.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;

        row.map(|row| {
            Ok::<InboxApprovalRow, StoreError>(InboxApprovalRow {
                approval: row_to_approval(&row)?,
                project_id: row.try_get("project_id")?,
                project_name: row.try_get("project_name")?,
                task_title: row.try_get("task_title")?,
                role_id: row.try_get("role_id")?,
                created_at: row.try_get("created_at")?,
                decided_at: row.try_get("decided_at")?,
            })
        })
        .transpose()
        .map_err(Into::into)
    }
}

impl ApprovalStore for Store {
    async fn get(&self, id: &ApprovalId) -> PortResult<Option<Approval>> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM approval WHERE id = ?"))
            .bind(id.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(StoreError::from)?;
        row.map(|row| row_to_approval(&row))
            .transpose()
            .map_err(Into::into)
    }

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

    async fn list_for_run(&self, run: &RunId) -> PortResult<Vec<Approval>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM approval WHERE run_id = ? ORDER BY created_at, id"
        ))
        .bind(run.as_str())
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
            "INSERT INTO approval
               (id, run_id, kind, status, payload, decision_comment, corrective_run_id)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               status = CASE
                   WHEN approval.status = 'pending' THEN excluded.status
                   ELSE approval.status END,
               payload = approval.payload,
               decision_comment = CASE
                   WHEN approval.status = 'pending' THEN excluded.decision_comment
                   ELSE approval.decision_comment END,
               corrective_run_id = COALESCE(approval.corrective_run_id,
                                            excluded.corrective_run_id),
               decided_at = CASE
                   WHEN approval.status = 'pending'
                    AND excluded.status IN ('granted', 'rejected')
                   THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                   ELSE approval.decided_at END",
        )
        .bind(approval.id.as_str())
        .bind(approval.run_id.as_str())
        .bind(approval.kind.as_str())
        .bind(approval.status.as_str())
        .bind(&approval.payload)
        .bind(&approval.decision_comment)
        .bind(approval.corrective_run_id.as_ref().map(|run| run.as_str()))
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

        a.reject_with_comment(Some("Corriger le focus".into()))
            .unwrap();
        s.save(&a).await.unwrap();
        assert!(s.list_pending().await.unwrap().is_empty());
        let back = ApprovalStore::get(&s, &a.id).await.unwrap().unwrap();
        assert_eq!(back.decision_comment.as_deref(), Some("Corriger le focus"));
    }

    #[tokio::test]
    async fn inbox_projection_carries_the_decision_context() {
        let (s, run) = test_fixtures::store_with_run().await;
        s.save(&approval("a1", run.as_str(), ApprovalKind::Review))
            .await
            .unwrap();

        let inbox = s.list_pending_for_inbox().await.unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].project_id, "p1");
        assert_eq!(inbox[0].project_name, "Mon App");
        assert_eq!(inbox[0].task_title, "Page de connexion");
        assert_eq!(inbox[0].role_id, "frontend");
        assert!(inbox[0].created_at.ends_with('Z'));
    }
}
