//! `ApproveSpec` — the owner approves a draft spec version. Owns the
//! cross-entity rule (core/spec.rs says so): the previously approved version
//! is superseded first, so the project never has two approved specs — the
//! partial unique index is the backstop, this is the mechanism.

use super::UseCaseError;
use crate::store::Store;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::SpecVersionId;
use latoile_core::ports::{EventLog, ProjectStore, SpecStore, TaskStore};
use latoile_core::SpecVersion;

pub struct ApproveSpec {
    store: Store,
}

impl ApproveSpec {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub async fn execute(&self, id: &SpecVersionId) -> Result<SpecVersion, UseCaseError> {
        // 2. Fetch: the draft to approve, and the currently approved one.
        let mut spec = self
            .store
            .spec_by_id(id)
            .await?
            .ok_or(UseCaseError::NotFound("spec version"))?;
        let previous = self.store.approved_for_project(&spec.project_id).await?;

        // 3. Domain. approve() refuses anything but a draft; supersede()
        // refuses anything but the approved one.
        spec.approve()?;
        let mut previous = previous;
        if let Some(prev) = previous.as_mut() {
            prev.supersede()?;
        }

        // 4. Persist, and the project is now specced.
        if let Some(prev) = previous {
            SpecStore::save(&self.store, &prev).await?;
        }
        SpecStore::save(&self.store, &spec).await?;
        let mut project = ProjectStore::get(&self.store, &spec.project_id)
            .await?
            .ok_or(UseCaseError::NotFound("project"))?;
        project.mark_specced();
        ProjectStore::save(&self.store, &project).await?;

        // Tasks waiting on the board materialize THIS spec (§5.2): bind
        // every task that has no spec yet, so the next dispatch passes the
        // spec-before-code guard. Tasks already bound keep their spec.
        let tasks = self.store.list_for_project(&spec.project_id).await?;
        for mut task in tasks {
            if task.spec_version_id.is_none() {
                task.bind_spec(spec.id.clone());
                TaskStore::save(&self.store, &task).await?;
            }
        }

        // 5. Journal.
        self.store
            .append(&NewEvent {
                project_id: spec.project_id.clone(),
                kind: EventKind::SpecApproved,
                payload: format!("{{\"spec_version_id\":\"{}\"}}", spec.id),
            })
            .await?;

        // 6. DTO.
        Ok(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ids::SpecVersionId;
    use latoile_core::{ProjectStatus, SpecStatus, SpecVersion};

    #[tokio::test]
    async fn approving_supersedes_the_previous_approved_spec() {
        let store = test_fixtures::store_with_approved_spec().await; // s1 approved
        let draft = SpecVersion::new(
            latoile_core::ids::SpecVersionId::new("s2").unwrap(),
            test_fixtures::PROJECT.clone(),
            2,
            "design/",
            None,
        )
        .unwrap();
        SpecStore::save(&store, &draft).await.unwrap();

        let approved = ApproveSpec::new(store.clone())
            .execute(&draft.id)
            .await
            .unwrap();

        assert_eq!(approved.status, SpecStatus::Approved);
        let old_id = SpecVersionId::new(test_fixtures::SPEC).unwrap();
        let old = store.spec_by_id(&old_id).await.unwrap().unwrap();
        assert_eq!(old.status, SpecStatus::Superseded);
        let project = ProjectStore::get(&store, &test_fixtures::PROJECT)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(project.status, ProjectStatus::Specced);
    }

    #[tokio::test]
    async fn approving_a_non_draft_is_refused() {
        let store = test_fixtures::store_with_approved_spec().await;
        let id = SpecVersionId::new(test_fixtures::SPEC).unwrap();
        // s1 is already approved — approving again must fail, and the store
        // must stay exactly as it was.
        assert!(ApproveSpec::new(store.clone()).execute(&id).await.is_err());
        let still = store.approved_for_project(&test_fixtures::PROJECT).await.unwrap();
        assert_eq!(still.unwrap().status, SpecStatus::Approved);
    }

    #[tokio::test]
    async fn approving_binds_the_spec_to_unbound_tasks() {
        let (store, _) = test_fixtures::store_with_task().await; // t1, bound to s1
        // A task created before the spec existed: no binding.
        let loose = latoile_core::Task::new(
            latoile_core::ids::TaskId::new("t9").unwrap(),
            test_fixtures::PROJECT.clone(),
            latoile_core::RoleId::new("backend").unwrap(),
            "Endpoint auth",
            "",
            1,
        )
        .unwrap();
        TaskStore::save(&store, &loose).await.unwrap();

        // Approve a second draft; it must become the tasks' spec.
        let draft = SpecVersion::new(
            SpecVersionId::new("s2").unwrap(),
            test_fixtures::PROJECT.clone(),
            2,
            "design/",
            None,
        )
        .unwrap();
        SpecStore::save(&store, &draft).await.unwrap();
        ApproveSpec::new(store.clone())
            .execute(&draft.id)
            .await
            .unwrap();

        let bound = TaskStore::get(&store, &loose.id).await.unwrap().unwrap();
        assert_eq!(
            bound.spec_version_id.as_ref().map(|s| s.as_str()),
            Some("s2")
        );
        // The fixture task was already bound to s1 — untouched.
        let t1 = TaskStore::get(&store, &latoile_core::ids::TaskId::new("t1").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            t1.spec_version_id.as_ref().map(|s| s.as_str()),
            Some("s1")
        );
    }
}
