//! `CreateProject` — register a project and open its Manager thread. The
//! conversation is created here because onboarding is the one place a
//! project is born; the ports have no conversation-creation surface, so this
//! use case takes the concrete `Store` (documented exception — if the port
//! grows one, this becomes generic like the others).

use super::UseCaseError;
use crate::store::Store;
use latoile_core::conversation::Conversation;
use latoile_core::ids::{ConversationId, ProjectId};
use latoile_core::ports::{ProjectStore, ProvisionWorkspaceInput, WorkspaceProvisioner};
use latoile_core::Project;

pub struct CreateProjectInput {
    pub name: String,
    pub slug: String,
    pub github_repo: String,
    pub work_branch: String,
    pub dev_command: Option<String>,
}

pub struct CreateProject<W> {
    store: Store,
    workspaces: W,
}

impl<W: WorkspaceProvisioner> CreateProject<W> {
    pub fn new(store: Store, workspaces: W) -> Self {
        Self { store, workspaces }
    }

    pub async fn execute(&self, input: CreateProjectInput) -> Result<Project, UseCaseError> {
        // 1. Validate identity before an adapter can create a checkout.
        Project::validate_identity(&input.name, &input.slug, &input.github_repo)?;

        // 2–3. Provision the repository; host paths and the default branch
        // are adapter-owned facts, never browser input.
        let provisioned = self
            .workspaces
            .provision(&ProvisionWorkspaceInput {
                repo: input.github_repo.clone(),
                slug: input.slug.clone(),
                work_branch: input.work_branch,
                dev_command: input.dev_command,
            })
            .await?;

        let mut project = Project::new(
            ProjectId::new(ulid::Ulid::new().to_string())?,
            input.name,
            input.slug,
            input.github_repo,
            provisioned.work_branch,
            provisioned.local_path,
            provisioned.dev_command,
        )?;
        project.default_branch = provisioned.default_branch;

        // 4. Persist: the project, then its single conversation.
        ProjectStore::save(&self.store, &project).await?;
        self.store
            .create_conversation(&Conversation::new(
                ConversationId::new(ulid::Ulid::new().to_string())?,
                project.id.clone(),
            ))
            .await?;

        // 5. No event: the domain declares no project-created kind, and this
        // layer does not invent one. 6. DTO.
        Ok(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use latoile_core::ports::{ConversationStore as _, PortResult, ProvisionedWorkspace};

    #[derive(Clone)]
    struct FakeWorkspace;

    impl WorkspaceProvisioner for FakeWorkspace {
        async fn provision(
            &self,
            input: &ProvisionWorkspaceInput,
        ) -> PortResult<ProvisionedWorkspace> {
            Ok(ProvisionedWorkspace {
                default_branch: "trunk".into(),
                work_branch: input.work_branch.clone(),
                local_path: format!("/srv/latoile/{}", input.slug),
                dev_command: input
                    .dev_command
                    .clone()
                    .unwrap_or_else(|| "pnpm dev -- --port $PORT".into()),
            })
        }
    }

    fn input() -> CreateProjectInput {
        CreateProjectInput {
            name: "Mon App".into(),
            slug: "mon-app".into(),
            github_repo: "salim4n/mon-app".into(),
            work_branch: "work".into(),
            dev_command: None,
        }
    }

    #[tokio::test]
    async fn a_project_is_created_with_its_conversation() {
        let store = Store::open_ephemeral().await.unwrap();
        let project = CreateProject::new(store.clone(), FakeWorkspace)
            .execute(input())
            .await
            .unwrap();

        assert!(store.get(&project.id).await.unwrap().is_some());
        assert!(store.for_project(&project.id).await.unwrap().is_some());
        assert_eq!(project.default_branch, "trunk");
    }

    #[tokio::test]
    async fn a_bad_repo_shape_is_refused_and_nothing_is_persisted() {
        let store = Store::open_ephemeral().await.unwrap();
        let mut bad = input();
        bad.github_repo = "no-slash".into();

        assert!(CreateProject::new(store.clone(), FakeWorkspace)
            .execute(bad)
            .await
            .is_err());
        assert!(store.list().await.unwrap().is_empty());
    }
}
