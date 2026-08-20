//! `StopPreview` — take the project's dev server down. The supervisor kills
//! the process, the domain closes the state machine (`stop` clears the pid),
//! and the freed slot is what lets the next `EnsurePreview` start fresh.

use super::UseCaseError;
use latoile_core::ids::ProjectId;
use latoile_core::ports::{PreviewStore, PreviewSupervisor};
use latoile_core::Preview;

pub struct StopPreview<PV, S> {
    previews: PV,
    supervisor: S,
}

impl<PV: PreviewStore, S: PreviewSupervisor> StopPreview<PV, S> {
    pub fn new(previews: PV, supervisor: S) -> Self {
        Self {
            previews,
            supervisor,
        }
    }

    /// `Ok(None)` when nothing was running — stopping what isn't there is
    /// success, matching the supervisor's own idempotence.
    pub async fn execute(&self, project: &ProjectId) -> Result<Option<Preview>, UseCaseError> {
        // 2. Fetch the active preview, if any.
        let Some(mut preview) = self.previews.active_for_project(project).await? else {
            return Ok(None);
        };

        // 3–4. Kill the process, then close the state machine and persist.
        self.supervisor.stop(&preview).await?;
        preview.stop()?;
        PreviewStore::save(&self.previews, &preview).await?;

        // 5. No event: the domain declares no preview-stopped kind.
        // 6. DTO.
        Ok(Some(preview))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ports::PortResult;
    use latoile_core::{PreviewId, PreviewStatus};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountingSupervisor(Arc<AtomicUsize>);

    impl PreviewSupervisor for CountingSupervisor {
        async fn ensure(
            &self,
            _p: &Preview,
            _cmd: &str,
            _working_dir: &str,
        ) -> PortResult<(u32, u16)> {
            Ok((4242, 4100))
        }
        async fn stop(&self, _p: &Preview) -> PortResult<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn an_active_preview_is_stopped_and_the_process_killed() {
        let store = test_fixtures::store_with_project().await;
        let mut preview = Preview::new(
            PreviewId::new("pr1").unwrap(),
            test_fixtures::PROJECT.clone(),
            4100,
            "work",
        );
        preview.mark_ready(4242).unwrap();
        store.save(&preview).await.unwrap();

        let stops = Arc::new(AtomicUsize::new(0));
        let uc = StopPreview::new(store.clone(), CountingSupervisor(stops.clone()));
        let stopped = uc.execute(&test_fixtures::PROJECT).await.unwrap().unwrap();

        assert_eq!(stopped.status, PreviewStatus::Stopped);
        assert_eq!(stopped.pid, None);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        // The slot is free: nothing is active for the project anymore.
        assert!(store
            .active_for_project(&test_fixtures::PROJECT)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn stopping_nothing_is_a_no_op() {
        let store = test_fixtures::store_with_project().await;
        let stops = Arc::new(AtomicUsize::new(0));
        let uc = StopPreview::new(store, CountingSupervisor(stops.clone()));
        assert!(uc.execute(&test_fixtures::PROJECT).await.unwrap().is_none());
        assert_eq!(stops.load(Ordering::SeqCst), 0);
    }
}
