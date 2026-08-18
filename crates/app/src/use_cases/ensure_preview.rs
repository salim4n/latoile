//! `EnsurePreview` — the project's dev server exists and serves the work
//! branch. Idempotent: a ready preview is returned untouched, a stale one is
//! recycled through the supervisor and refreshed, a missing (or dead) one is
//! created. `Preview::refresh()` is only ever called on a stale preview —
//! anything else would be a domain refusal.

use super::UseCaseError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{PreviewId, ProjectId};
use latoile_core::ports::{EventLog, PreviewStore, PreviewSupervisor, ProjectStore};
use latoile_core::{Preview, PreviewStatus};

pub struct EnsuredPreview {
    pub preview: Preview,
    /// True when the supervisor was (re)started by this call.
    pub recycled: bool,
}

pub struct EnsurePreview<PJ, PV, S, E> {
    projects: PJ,
    previews: PV,
    supervisor: S,
    events: E,
}

impl<PJ: ProjectStore, PV: PreviewStore, S: PreviewSupervisor, E: EventLog>
    EnsurePreview<PJ, PV, S, E>
{
    pub fn new(projects: PJ, previews: PV, supervisor: S, events: E) -> Self {
        Self {
            projects,
            previews,
            supervisor,
            events,
        }
    }

    pub async fn execute(&self, project_id: &ProjectId) -> Result<EnsuredPreview, UseCaseError> {
        // 2. Fetch.
        let project = self
            .projects
            .get(project_id)
            .await?
            .ok_or(UseCaseError::NotFound("project"))?;

        match self.previews.active_for_project(project_id).await? {
            // Starting or ready: nothing to do.
            Some(preview) if preview.status != PreviewStatus::Stale => Ok(EnsuredPreview {
                preview,
                recycled: false,
            }),
            // Stale: recycle the dev server, then refresh (Stale → Ready is
            // the only path `refresh` accepts).
            Some(mut preview) => {
                let (pid, port) = self
                    .supervisor
                    .ensure(&preview, &project.dev_command, &project.local_path)
                    .await?;
                preview.refresh()?;
                preview.port = port;
                preview.pid = Some(pid);
                self.finish(preview, project_id, true).await
            }
            // None (never started, errored, or stopped): a fresh preview.
            None => {
                let mut preview = Preview::new(
                    PreviewId::new(ulid::Ulid::new().to_string())?,
                    project_id.clone(),
                    0, // placeholder — the supervisor allocates the real port
                    project.work_branch.clone(),
                );
                let (pid, port) = self
                    .supervisor
                    .ensure(&preview, &project.dev_command, &project.local_path)
                    .await?;
                preview.port = port;
                preview.mark_ready(pid)?;
                self.finish(preview, project_id, true).await
            }
        }
    }

    /// Steps 4–6 shared by both branches: persist, journal, return the DTO.
    async fn finish(
        &self,
        preview: Preview,
        project_id: &ProjectId,
        recycled: bool,
    ) -> Result<EnsuredPreview, UseCaseError> {
        self.previews.save(&preview).await?;
        self.events
            .append(&NewEvent {
                project_id: project_id.clone(),
                kind: EventKind::PreviewReady,
                payload: format!(
                    "{{\"preview_id\":\"{}\",\"port\":{}}}",
                    preview.id, preview.port
                ),
            })
            .await?;
        Ok(EnsuredPreview { preview, recycled })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use crate::store::Store;
    use latoile_core::ports::PortResult;

    /// A fake supervisor: allocates pid 4242 and port 4100, no process.
    struct FakeSupervisor;

    impl PreviewSupervisor for FakeSupervisor {
        async fn ensure(
            &self,
            _p: &Preview,
            _cmd: &str,
            working_dir: &str,
        ) -> PortResult<(u32, u16)> {
            assert_eq!(working_dir, "/srv/latoile/mon-app");
            Ok((4242, 4100))
        }
        async fn stop(&self, _p: &Preview) -> PortResult<()> {
            Ok(())
        }
    }

    fn use_case(store: &Store) -> EnsurePreview<Store, Store, FakeSupervisor, Store> {
        EnsurePreview::new(store.clone(), store.clone(), FakeSupervisor, store.clone())
    }

    #[tokio::test]
    async fn a_project_without_preview_gets_a_ready_one() {
        let store = test_fixtures::store_with_project().await;
        let out = use_case(&store)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap();

        assert!(out.recycled);
        assert_eq!(out.preview.status, PreviewStatus::Ready);
        assert_eq!(out.preview.port, 4100);
        assert_eq!(out.preview.pid, Some(4242));

        let events = store.since(&test_fixtures::PROJECT, 0).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1.kind, EventKind::PreviewReady);
    }

    #[tokio::test]
    async fn a_ready_preview_is_returned_untouched() {
        let store = test_fixtures::store_with_project().await;
        let first = use_case(&store)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap();
        let second = use_case(&store)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap();

        assert!(!second.recycled);
        assert_eq!(second.preview, first.preview);
        // No second event.
        assert_eq!(
            store.since(&test_fixtures::PROJECT, 0).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn a_stale_preview_is_recycled_and_refreshed() {
        let store = test_fixtures::store_with_project().await;
        let mut preview = use_case(&store)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap()
            .preview;
        preview.mark_stale().unwrap();
        PreviewStore::save(&store, &preview).await.unwrap();

        let out = use_case(&store)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap();

        assert!(out.recycled);
        assert_eq!(out.preview.id, preview.id);
        assert_eq!(out.preview.status, PreviewStatus::Ready);
    }

    #[tokio::test]
    async fn a_dead_preview_frees_the_slot_for_a_fresh_one() {
        let store = test_fixtures::store_with_project().await;
        let mut preview = use_case(&store)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap()
            .preview;
        preview.fail().unwrap();
        PreviewStore::save(&store, &preview).await.unwrap();
        assert!(store
            .active_for_project(&test_fixtures::PROJECT)
            .await
            .unwrap()
            .is_none());

        let out = use_case(&store)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap();
        assert_eq!(out.preview.status, PreviewStatus::Ready);
        assert_ne!(out.preview.id, preview.id);
    }

    #[tokio::test]
    async fn an_unknown_project_is_refused() {
        let store = test_fixtures::store_with_project().await;
        assert!(use_case(&store)
            .execute(&ProjectId::new("ghost").unwrap())
            .await
            .is_err());
    }
}
