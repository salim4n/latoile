//! Project directory resolution for the agent channel: the project's
//! `local_path` from the store — the checkout the code lives in. Run dirs go
//! run → task → project. Unknown anything resolves to `None`; the channel
//! turns that into a clean error before spawning.

use latoile_agents::ProjectDirs;
use latoile_app::store::Store;
use latoile_core::ids::ProjectId;
use latoile_core::ports::{ProjectStore, RunStore, TaskStore};
use latoile_core::Run;
use std::path::{Component, Path, PathBuf};

#[derive(Clone)]
pub struct StoreDirs {
    store: Store,
    workspace: PathBuf,
}

impl StoreDirs {
    pub fn new(store: Store, workspace: PathBuf) -> Self {
        Self { store, workspace }
    }

    /// Absolute paths are explicit operator choices. Relative paths are
    /// project locations under the configured workspace; parent traversal
    /// is refused so a repository name cannot escape that root.
    fn resolve(&self, path: &str) -> Option<PathBuf> {
        let path = Path::new(path);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        if path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return None;
        }
        Some(self.workspace.join(path))
    }
}

#[allow(clippy::manual_async_fn)] // the trait needs the explicit `+ Send`
impl ProjectDirs for StoreDirs {
    fn manager_dir<'a>(
        &'a self,
        project: &'a ProjectId,
    ) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'a {
        async move {
            ProjectStore::get(&self.store, project)
                .await
                .ok()
                .flatten()
                .and_then(|p| self.resolve(&p.local_path))
        }
    }

    fn run_dir<'a>(
        &'a self,
        run: &'a Run,
    ) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'a {
        async move {
            let run = RunStore::get(&self.store, &run.id).await.ok().flatten()?;
            let task = TaskStore::get(&self.store, &run.task_id)
                .await
                .ok()
                .flatten()?;
            ProjectStore::get(&self.store, &task.project_id)
                .await
                .ok()
                .flatten()
                .and_then(|p| self.resolve(&p.local_path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use latoile_core::ids::{RoleId, RunId, SpecVersionId, TaskId};
    use latoile_core::{Project, Run as RunEntity, SpecVersion, Task, TriggeredBy};

    async fn store_with_run(local_path: &str) -> (Store, RunEntity, ProjectId) {
        let store = Store::open_ephemeral().await.unwrap();
        let project = Project::new(
            ProjectId::new("p1").unwrap(),
            "Mon App",
            "mon-app",
            "salim4n/mon-app",
            "work",
            local_path,
            "pnpm dev",
        )
        .unwrap();
        ProjectStore::save(&store, &project).await.unwrap();

        let mut spec = SpecVersion::new(
            SpecVersionId::new("s1").unwrap(),
            project.id.clone(),
            1,
            "design/",
            None,
        )
        .unwrap();
        spec.approve().unwrap();
        latoile_core::ports::SpecStore::save(&store, &spec)
            .await
            .unwrap();

        let mut task = Task::new(
            TaskId::new("t1").unwrap(),
            project.id.clone(),
            RoleId::new("frontend").unwrap(),
            "T",
            "d",
            0,
        )
        .unwrap();
        task.bind_spec(spec.id.clone());
        TaskStore::save(&store, &task).await.unwrap();

        let run = RunEntity::new(
            RunId::new("r1").unwrap(),
            task.id,
            RoleId::new("frontend").unwrap(),
            TriggeredBy::Manager,
        );
        RunStore::save(&store, &run).await.unwrap();
        (store, run, project.id)
    }

    #[tokio::test]
    async fn a_project_resolves_to_its_local_path() {
        let (store, _, project) = store_with_run("/srv/latoile/mon-app").await;
        let dirs = StoreDirs::new(store, PathBuf::from("/srv/latoile/workspace"));
        assert_eq!(
            dirs.manager_dir(&project).await,
            Some(PathBuf::from("/srv/latoile/mon-app"))
        );
    }

    #[tokio::test]
    async fn a_run_resolves_through_its_task() {
        let (store, run, _) = store_with_run("/srv/latoile/mon-app").await;
        let dirs = StoreDirs::new(store, PathBuf::from("/srv/latoile/workspace"));
        assert_eq!(
            dirs.run_dir(&run).await,
            Some(PathBuf::from("/srv/latoile/mon-app"))
        );
    }

    #[tokio::test]
    async fn an_unknown_project_resolves_to_nothing() {
        let store = Store::open_ephemeral().await.unwrap();
        let dirs = StoreDirs::new(store, PathBuf::from("/srv/latoile/workspace"));
        assert_eq!(
            dirs.manager_dir(&ProjectId::new("ghost").unwrap()).await,
            None
        );
    }

    #[tokio::test]
    async fn a_relative_project_path_stays_under_the_workspace() {
        let (store, _, project) = store_with_run("salim4n/mon-app").await;
        let dirs = StoreDirs::new(store, PathBuf::from("/srv/latoile/workspace"));
        assert_eq!(
            dirs.manager_dir(&project).await,
            Some(PathBuf::from("/srv/latoile/workspace/salim4n/mon-app"))
        );
    }

    #[tokio::test]
    async fn a_relative_path_cannot_escape_the_workspace() {
        let store = Store::open_ephemeral().await.unwrap();
        let dirs = StoreDirs::new(store, PathBuf::from("/srv/latoile/workspace"));
        assert_eq!(dirs.resolve("../outside"), None);
    }
}
