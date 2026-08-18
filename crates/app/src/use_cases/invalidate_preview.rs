//! Mark a ready project preview stale after a frontend executor commit. The
//! next `EnsurePreview` call must recycle the process before visual capture;
//! otherwise a long-lived dev server may keep serving the previous run's code.

use super::UseCaseError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::ProjectId;
use latoile_core::ports::{EventLog, PreviewStore};
use latoile_core::PreviewStatus;

pub struct InvalidatePreview<P, E> {
    previews: P,
    events: E,
}

impl<P: PreviewStore, E: EventLog> InvalidatePreview<P, E> {
    pub fn new(previews: P, events: E) -> Self {
        Self { previews, events }
    }

    /// Returns true only when this call performed the ready → stale transition.
    pub async fn execute(&self, project: &ProjectId) -> Result<bool, UseCaseError> {
        let Some(mut preview) = self.previews.active_for_project(project).await? else {
            return Ok(false);
        };
        if preview.status != PreviewStatus::Ready {
            return Ok(false);
        }
        preview.mark_stale()?;
        self.previews.save(&preview).await?;
        self.events
            .append(&NewEvent {
                project_id: project.clone(),
                kind: EventKind::PreviewStale,
                payload: serde_json::json!({
                    "preview_id": preview.id.as_str(),
                    "reason": "frontend_run_finished",
                })
                .to_string(),
            })
            .await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ids::PreviewId;
    use latoile_core::ports::{EventLog, PreviewStore};
    use latoile_core::Preview;

    #[tokio::test]
    async fn a_ready_preview_becomes_stale_once_before_recapture() {
        let store = test_fixtures::store_with_project().await;
        let mut preview = Preview::new(
            PreviewId::new("preview-1").unwrap(),
            test_fixtures::PROJECT.clone(),
            4100,
            "work",
        );
        preview.mark_ready(4242).unwrap();
        PreviewStore::save(&store, &preview).await.unwrap();
        let use_case = InvalidatePreview::new(store.clone(), store.clone());

        assert!(use_case.execute(&test_fixtures::PROJECT).await.unwrap());
        assert!(!use_case.execute(&test_fixtures::PROJECT).await.unwrap());
        let stored = PreviewStore::active_for_project(&store, &test_fixtures::PROJECT)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, PreviewStatus::Stale);
        let events = EventLog::since(&store, &test_fixtures::PROJECT, 0)
            .await
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|(_, event)| event.kind == EventKind::PreviewStale)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn no_existing_preview_is_an_idempotent_noop() {
        let store = test_fixtures::store_with_project().await;
        assert!(!InvalidatePreview::new(store.clone(), store.clone())
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap());
        assert!(EventLog::since(&store, &test_fixtures::PROJECT, 0)
            .await
            .unwrap()
            .is_empty());
    }
}
