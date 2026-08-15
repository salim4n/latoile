//! `event` table — the append-only journal. `seq` is assigned by SQLite
//! (AUTOINCREMENT) and is the only SSE cursor (contract §4). Nothing here
//! updates or deletes.

use super::{unknown_variant, Store, StoreError};
use latoile_core::event::EventKind;
use latoile_core::ports::{EventLog, PortError, PortResult};
use latoile_core::{NewEvent, ProjectId};
use sqlx::Row;

fn parse_kind(raw: &str) -> Result<EventKind, StoreError> {
    Ok(match raw {
        "spec_version_created" => EventKind::SpecVersionCreated,
        "spec_approved" => EventKind::SpecApproved,
        "task_ready" => EventKind::TaskReady,
        "run_started" => EventKind::RunStarted,
        "run_blocked" => EventKind::RunBlocked,
        "run_finished" => EventKind::RunFinished,
        "approval_requested" => EventKind::ApprovalRequested,
        "approval_granted" => EventKind::ApprovalGranted,
        "approval_rejected" => EventKind::ApprovalRejected,
        "preview_ready" => EventKind::PreviewReady,
        "preview_stale" => EventKind::PreviewStale,
        "preview_error" => EventKind::PreviewError,
        "message_posted" => EventKind::MessagePosted,
        other => return Err(unknown_variant("event kind", other)),
    })
}

impl EventLog for Store {
    async fn append(&self, event: &NewEvent) -> PortResult<u64> {
        let result = sqlx::query(
            "INSERT INTO event (project_id, kind, payload) VALUES (?, ?, ?)",
        )
        .bind(event.project_id.as_str())
        .bind(event.kind.as_str())
        .bind(&event.payload)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        u64::try_from(result.last_insert_rowid())
            .map_err(|_| PortError("negative event seq".into()))
    }

    /// Everything after `after_seq`, oldest first — the SSE resume path.
    async fn since(
        &self,
        project: &ProjectId,
        after_seq: u64,
    ) -> PortResult<Vec<(u64, NewEvent)>> {
        let rows = sqlx::query(
            "SELECT seq, project_id, kind, payload FROM event
             WHERE project_id = ? AND seq > ? ORDER BY seq ASC",
        )
        .bind(project.as_str())
        .bind(i64::try_from(after_seq).unwrap_or(i64::MAX))
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(|r| {
                Ok((
                    u64::try_from(r.try_get::<i64, _>("seq")?)
                        .map_err(|_| StoreError::CorruptRow("negative seq".into()))?,
                    NewEvent {
                        project_id: ProjectId::new(r.try_get::<String, _>("project_id")?)?,
                        kind: parse_kind(&r.try_get::<String, _>("kind")?)?,
                        payload: r.try_get("payload")?,
                    },
                ))
            })
            .collect::<Result<Vec<_>, StoreError>>()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;

    fn event(kind: EventKind, payload: &str) -> NewEvent {
        NewEvent {
            project_id: test_fixtures::PROJECT.clone(),
            kind,
            payload: payload.into(),
        }
    }

    #[tokio::test]
    async fn seq_is_monotonic_and_assigned_by_the_store() {
        let s = test_fixtures::store_with_project().await;
        let a = s.append(&event(EventKind::TaskReady, "{}")).await.unwrap();
        let b = s.append(&event(EventKind::RunStarted, "{}")).await.unwrap();
        let c = s
            .append(&event(EventKind::RunFinished, "{}"))
            .await
            .unwrap();
        assert!(a < b && b < c, "{a} < {b} < {c}");
    }

    #[tokio::test]
    async fn since_replays_only_what_was_missed() {
        let s = test_fixtures::store_with_project().await;
        let first = s.append(&event(EventKind::TaskReady, "1")).await.unwrap();
        s.append(&event(EventKind::RunStarted, "2")).await.unwrap();
        s.append(&event(EventKind::RunFinished, "3")).await.unwrap();

        let missed = s.since(&test_fixtures::PROJECT, first).await.unwrap();
        assert_eq!(missed.len(), 2);
        assert_eq!(missed[0].1.kind, EventKind::RunStarted);
        assert_eq!(missed[1].1.kind, EventKind::RunFinished);
        assert!(missed[0].0 > first);

        // Resuming from zero replays everything.
        assert_eq!(s.since(&test_fixtures::PROJECT, 0).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn events_are_scoped_to_their_project() {
        let s = test_fixtures::store_with_project().await;
        s.append(&event(EventKind::TaskReady, "{}"))
            .await
            .unwrap();
        let other = ProjectId::new("other").unwrap();
        assert!(s.since(&other, 0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn every_kind_round_trips() {
        let s = test_fixtures::store_with_project().await;
        let kinds = [
            EventKind::SpecVersionCreated,
            EventKind::SpecApproved,
            EventKind::TaskReady,
            EventKind::RunStarted,
            EventKind::RunBlocked,
            EventKind::RunFinished,
            EventKind::ApprovalRequested,
            EventKind::ApprovalGranted,
            EventKind::ApprovalRejected,
            EventKind::PreviewReady,
            EventKind::PreviewStale,
            EventKind::PreviewError,
            EventKind::MessagePosted,
        ];
        for kind in kinds {
            s.append(&event(kind, "{}")).await.unwrap();
        }
        let back = s.since(&test_fixtures::PROJECT, 0).await.unwrap();
        assert_eq!(
            back.iter().map(|(_, e)| e.kind).collect::<Vec<_>>(),
            kinds
        );
    }
}
