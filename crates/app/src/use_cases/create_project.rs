//! `CreateProject` — register a project and open its Manager thread. The
//! conversation is created here because onboarding is the one place a
//! project is born; the ports have no conversation-creation surface, so this
//! use case takes the concrete `Store` (documented exception — if the port
//! grows one, this becomes generic like the others).

use super::UseCaseError;
use crate::store::Store;
use latoile_core::conversation::Conversation;
use latoile_core::ids::{ConversationId, ProjectId};
use latoile_core::ports::ProjectStore;
use latoile_core::Project;

pub struct CreateProjectInput {
    pub name: String,
    pub slug: String,
    pub github_repo: String,
    pub work_branch: String,
    pub local_path: String,
    pub dev_command: String,
}

pub struct CreateProject {
    store: Store,
}

impl CreateProject {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub async fn execute(&self, input: CreateProjectInput) -> Result<Project, UseCaseError> {
        // 1–3. Validate and build through the domain constructor (name/slug
        // non-empty, repo looks like owner/name).
        let project = Project::new(
            ProjectId::new(ulid::Ulid::new().to_string())?,
            input.name,
            input.slug,
            input.github_repo,
            input.work_branch,
            input.local_path,
            input.dev_command,
        )?;

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
    use latoile_core::ports::ConversationStore as _;

    fn input() -> CreateProjectInput {
        CreateProjectInput {
            name: "Mon App".into(),
            slug: "mon-app".into(),
            github_repo: "salim4n/mon-app".into(),
            work_branch: "work".into(),
            local_path: "/srv/latoile/mon-app".into(),
            dev_command: "pnpm dev --port $PORT".into(),
        }
    }

    #[tokio::test]
    async fn a_project_is_created_with_its_conversation() {
        let store = Store::open_ephemeral().await.unwrap();
        let project = CreateProject::new(store.clone())
            .execute(input())
            .await
            .unwrap();

        assert!(store.get(&project.id).await.unwrap().is_some());
        assert!(store.for_project(&project.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_bad_repo_shape_is_refused_and_nothing_is_persisted() {
        let store = Store::open_ephemeral().await.unwrap();
        let mut bad = input();
        bad.github_repo = "no-slash".into();

        assert!(CreateProject::new(store.clone()).execute(bad).await.is_err());
        assert!(store.list().await.unwrap().is_empty());
    }
}
