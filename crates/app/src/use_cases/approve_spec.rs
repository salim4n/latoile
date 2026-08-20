//! `ApproveSpec` — the owner approves a draft spec version. Owns the
//! cross-entity rule (core/spec.rs says so): the previously approved version
//! is superseded first, so the project never has two approved specs — the
//! partial unique index is the backstop, this is the mechanism.

use super::UseCaseError;
use crate::store::Store;
use latoile_core::SpecVersion;
use latoile_core::ids::SpecVersionId;
use latoile_core::ports::{AgentChannel, ProjectStore, SpecStore};

pub struct ApproveSpec<A> {
    store: Store,
    agents: A,
}

impl<A: AgentChannel> ApproveSpec<A> {
    pub fn new(store: Store, agents: A) -> Self {
        Self { store, agents }
    }

    pub async fn execute(&self, id: &SpecVersionId) -> Result<SpecVersion, UseCaseError> {
        // 2. Fetch: the draft to approve, and the currently approved one.
        let mut spec = self
            .store
            .spec_by_id(id)
            .await?
            .ok_or(UseCaseError::NotFound("spec version"))?;
        if spec.provenance.is_none() {
            return Err(latoile_core::DomainError::Invariant(
                "only a complete immutable Architect package can be approved",
            )
            .into());
        }
        let project_id = spec.project_id.clone();
        let verification = self
            .agents
            .verify_architecture_package(&project_id, &spec)
            .await?;

        // 3. Domain. approve() refuses anything but a draft; supersede()
        // refuses anything but the approved one.
        spec.approve(&verification)?;
        let mut previous = self.store.approved_for_project(&project_id).await?;
        if let Some(prev) = previous.as_mut() {
            prev.supersede()?;
        }

        // 4. Persist every approval consequence in one SQLite transaction.
        let mut project = ProjectStore::get(&self.store, &project_id)
            .await?
            .ok_or(UseCaseError::NotFound("project"))?;
        project.mark_specced();
        self.store
            .approve_spec_atomically(&spec, previous.as_ref(), &project)
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
    use latoile_core::ports::{ManagerReply, PortResult, TaskStore};
    use latoile_core::{ProjectStatus, SpecStatus, SpecVersion};

    #[derive(Clone)]
    struct FakeAgents {
        valid: bool,
    }

    impl AgentChannel for FakeAgents {
        async fn tell_manager(
            &self,
            _project: &latoile_core::ProjectId,
            _message: &str,
        ) -> PortResult<ManagerReply> {
            unimplemented!()
        }

        async fn verify_architecture_package(
            &self,
            _project: &latoile_core::ProjectId,
            spec: &SpecVersion,
        ) -> PortResult<latoile_core::ArchitecturePackageValidation> {
            let mut verification = test_fixtures::test_verification(spec);
            verification.valid = self.valid;
            Ok(verification)
        }

        async fn start_run(
            &self,
            _project: &latoile_core::ProjectId,
            _run: &latoile_core::Run,
            _prompt: &str,
        ) -> PortResult<String> {
            unimplemented!()
        }

        async fn cancel_run(&self, _run: &latoile_core::RunId) -> PortResult<()> {
            Ok(())
        }
    }

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
        let mut draft = draft;
        test_fixtures::attach_test_provenance(&mut draft);
        SpecStore::save(&store, &draft).await.unwrap();

        let approved = ApproveSpec::new(store.clone(), FakeAgents { valid: true })
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
        assert!(
            ApproveSpec::new(store.clone(), FakeAgents { valid: true })
                .execute(&id)
                .await
                .is_err()
        );
        let still = store
            .approved_for_project(&test_fixtures::PROJECT)
            .await
            .unwrap();
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
        let mut draft = draft;
        test_fixtures::attach_test_provenance(&mut draft);
        SpecStore::save(&store, &draft).await.unwrap();
        ApproveSpec::new(store.clone(), FakeAgents { valid: true })
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
        assert_eq!(t1.spec_version_id.as_ref().map(|s| s.as_str()), Some("s1"));
    }

    #[tokio::test]
    async fn invalid_package_cannot_partially_change_the_database() {
        let store = test_fixtures::store_with_approved_spec().await;
        let mut draft = SpecVersion::new(
            SpecVersionId::new("s2").unwrap(),
            test_fixtures::PROJECT.clone(),
            2,
            "design/v0002-test/",
            None,
        )
        .unwrap();
        test_fixtures::attach_test_provenance(&mut draft);
        SpecStore::save(&store, &draft).await.unwrap();

        assert!(
            ApproveSpec::new(store.clone(), FakeAgents { valid: false })
                .execute(&draft.id)
                .await
                .is_err()
        );
        assert_eq!(
            store.spec_by_id(&draft.id).await.unwrap().unwrap().status,
            SpecStatus::Draft
        );
        assert_eq!(
            store
                .approved_for_project(&test_fixtures::PROJECT)
                .await
                .unwrap()
                .unwrap()
                .id
                .as_str(),
            "s1"
        );
    }
}
