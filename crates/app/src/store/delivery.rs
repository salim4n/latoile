//! Verified work-branch push and Pull Request evidence.

use super::{unknown_variant, Store, StoreError};
use latoile_core::ports::{DeliveryStore, PortResult};
use latoile_core::{Delivery, DeliveryStatus, ProjectId};
use sqlx::Row;

fn parse_status(raw: &str) -> Result<DeliveryStatus, StoreError> {
    match raw {
        "pushed" => Ok(DeliveryStatus::Pushed),
        "pull_request_open" => Ok(DeliveryStatus::PullRequestOpen),
        other => Err(unknown_variant("delivery status", other)),
    }
}

impl DeliveryStore for Store {
    async fn get_for_project(&self, project: &ProjectId) -> PortResult<Option<Delivery>> {
        let row = sqlx::query(
            "SELECT project_id, work_branch, local_sha, remote_sha, status, pull_request_url
             FROM delivery WHERE project_id = ?",
        )
        .bind(project.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.map(|row| {
            Ok::<Delivery, StoreError>(Delivery {
                project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)?,
                work_branch: row.try_get("work_branch")?,
                local_sha: row.try_get("local_sha")?,
                remote_sha: row.try_get("remote_sha")?,
                status: parse_status(&row.try_get::<String, _>("status")?)?,
                pull_request_url: row.try_get("pull_request_url")?,
            })
        })
        .transpose()
        .map_err(Into::into)
    }

    async fn save(&self, delivery: &Delivery) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO delivery
               (project_id, work_branch, local_sha, remote_sha, status, pull_request_url)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(project_id) DO UPDATE SET
               work_branch = excluded.work_branch,
               local_sha = excluded.local_sha,
               remote_sha = excluded.remote_sha,
               status = excluded.status,
               pull_request_url = excluded.pull_request_url,
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(delivery.project_id.as_str())
        .bind(&delivery.work_branch)
        .bind(&delivery.local_sha)
        .bind(&delivery.remote_sha)
        .bind(delivery.status.as_str())
        .bind(&delivery.pull_request_url)
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

    #[tokio::test]
    async fn pushed_evidence_upgrades_to_the_open_pull_request() {
        let store = test_fixtures::store_with_project().await;
        let mut delivery =
            Delivery::pushed(test_fixtures::PROJECT.clone(), "work", "abc", "abc").unwrap();
        store.save(&delivery).await.unwrap();
        assert_eq!(
            store
                .get_for_project(&test_fixtures::PROJECT)
                .await
                .unwrap()
                .unwrap()
                .status,
            DeliveryStatus::Pushed
        );

        delivery
            .attach_pull_request("https://github.com/salim4n/mon-app/pull/1")
            .unwrap();
        store.save(&delivery).await.unwrap();
        assert_eq!(
            store
                .get_for_project(&test_fixtures::PROJECT)
                .await
                .unwrap()
                .unwrap(),
            delivery
        );
    }
}
