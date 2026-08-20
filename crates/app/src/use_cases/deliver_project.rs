//! Explicit owner-controlled project delivery: prove every selected task was
//! approved, verify and push the clean work branch, then find or open its PR.
//! Nothing in this use case merges.

use super::UseCaseError;
use latoile_core::error::DomainError;
use latoile_core::ids::ProjectId;
use latoile_core::ports::{
    ApprovalStore, DeliveryStore, GitHubClient, ProjectStore, PublishWorkBranchInput, RunStore,
    TaskStore, WorkBranchPublisher,
};
use latoile_core::{ApprovalKind, ApprovalStatus, Delivery, RunStatus, TaskStatus};

pub struct DeliverProject<P, T, R, A, D, W, G> {
    projects: P,
    tasks: T,
    runs: R,
    approvals: A,
    deliveries: D,
    publisher: W,
    github: G,
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::store::{test_fixtures, Store};
    use latoile_core::ids::{ApprovalId, RoleId, RunId};
    use latoile_core::ports::{
        DeliveryStore, PortError, PortResult, PublishedWorkBranch, RepoInfo,
    };
    use latoile_core::{Approval, DeliveryStatus, Run, TriggeredBy};
    use std::sync::{Arc, Mutex};

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PR: &str = "https://github.com/salim4n/mon-app/pull/7";

    #[derive(Clone)]
    struct FakeDelivery {
        open_pr: Arc<Mutex<Option<String>>>,
        pushes: Arc<Mutex<Vec<PublishWorkBranchInput>>>,
        opens: Arc<Mutex<usize>>,
        publish_error: bool,
        open_error: bool,
    }

    impl FakeDelivery {
        fn new(existing: Option<&str>) -> Self {
            Self {
                open_pr: Arc::new(Mutex::new(existing.map(str::to_string))),
                pushes: Arc::new(Mutex::new(Vec::new())),
                opens: Arc::new(Mutex::new(0)),
                publish_error: false,
                open_error: false,
            }
        }
    }

    impl WorkBranchPublisher for FakeDelivery {
        async fn verify_and_push(
            &self,
            input: &PublishWorkBranchInput,
        ) -> PortResult<PublishedWorkBranch> {
            self.pushes.lock().unwrap().push(input.clone());
            if self.publish_error {
                return Err(PortError("unsafe worktree".into()));
            }
            Ok(PublishedWorkBranch {
                work_branch: input.work_branch.clone(),
                local_sha: SHA.into(),
                remote_sha: SHA.into(),
            })
        }
    }

    impl GitHubClient for FakeDelivery {
        async fn list_repos(&self) -> PortResult<Vec<RepoInfo>> {
            Ok(Vec::new())
        }

        async fn open_pull_request(&self, _: &str, _: &str, _: &str) -> PortResult<String> {
            *self.opens.lock().unwrap() += 1;
            if self.open_error {
                return Err(PortError("GitHub unavailable".into()));
            }
            *self.open_pr.lock().unwrap() = Some(PR.into());
            Ok(PR.into())
        }

        async fn find_open_pull_request(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> PortResult<Option<String>> {
            Ok(self.open_pr.lock().unwrap().clone())
        }
    }

    async fn approved_project() -> Store {
        let (store, task_id) = test_fixtures::store_with_task().await;
        let mut task = latoile_core::ports::TaskStore::get(&store, &task_id)
            .await
            .unwrap()
            .unwrap();
        task.start().unwrap();

        let mut executor = Run::new(
            RunId::new("executor-1").unwrap(),
            task.id.clone(),
            RoleId::new("frontend").unwrap(),
            TriggeredBy::Manager,
        );
        executor.begin().unwrap();
        executor.finish("implemented").unwrap();
        executor
            .attach_evidence(Some(SHA.into()), Some(SHA.into()), "{}".into())
            .unwrap();
        latoile_core::ports::RunStore::save(&store, &executor)
            .await
            .unwrap();

        task.submit_for_review().unwrap();
        let mut reviewer = Run::new(
            RunId::new("reviewer-1").unwrap(),
            task.id.clone(),
            RoleId::new("reviewer").unwrap(),
            TriggeredBy::Manager,
        );
        reviewer.begin().unwrap();
        reviewer.finish("approved").unwrap();
        latoile_core::ports::RunStore::save(&store, &reviewer)
            .await
            .unwrap();
        let mut approval = Approval::new(
            ApprovalId::new("review-1").unwrap(),
            reviewer.id,
            ApprovalKind::Review,
            "{}".into(),
        );
        approval.grant().unwrap();
        ApprovalStore::save(&store, &approval).await.unwrap();
        task.approve(&approval).unwrap();
        latoile_core::ports::TaskStore::save(&store, &task)
            .await
            .unwrap();
        store
    }

    fn use_case(
        store: &Store,
        adapter: &FakeDelivery,
    ) -> DeliverProject<Store, Store, Store, Store, Store, FakeDelivery, FakeDelivery> {
        DeliverProject::new(
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            store.clone(),
            adapter.clone(),
            adapter.clone(),
        )
    }

    #[tokio::test]
    async fn pushes_verified_sha_and_opens_one_idempotent_pull_request() {
        let store = approved_project().await;
        let adapter = FakeDelivery::new(None);
        let delivered = use_case(&store, &adapter)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap();
        assert_eq!(delivered.status, DeliveryStatus::PullRequestOpen);
        assert_eq!(delivered.local_sha, SHA);
        assert_eq!(delivered.remote_sha, SHA);
        assert_eq!(delivered.pull_request_url.as_deref(), Some(PR));
        assert_eq!(adapter.opens.lock().unwrap().to_owned(), 1);
        assert_eq!(adapter.pushes.lock().unwrap()[0].approved_shas, [SHA]);

        let retried = use_case(&store, &adapter)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap();
        assert_eq!(retried.pull_request_url, delivered.pull_request_url);
        assert_eq!(*adapter.opens.lock().unwrap(), 1);
        assert_eq!(adapter.pushes.lock().unwrap().len(), 2);
        assert_eq!(
            DeliveryStore::get_for_project(&store, &test_fixtures::PROJECT)
                .await
                .unwrap(),
            Some(retried)
        );
    }

    #[tokio::test]
    async fn reuses_an_already_open_pull_request_without_creating_one() {
        let store = approved_project().await;
        let adapter = FakeDelivery::new(Some(PR));
        let delivered = use_case(&store, &adapter)
            .execute(&test_fixtures::PROJECT)
            .await
            .unwrap();
        assert_eq!(delivered.pull_request_url.as_deref(), Some(PR));
        assert_eq!(*adapter.opens.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn refuses_unapproved_work_before_touching_git() {
        let (store, _) = test_fixtures::store_with_task().await;
        let adapter = FakeDelivery::new(None);
        assert!(use_case(&store, &adapter)
            .execute(&test_fixtures::PROJECT)
            .await
            .is_err());
        assert!(adapter.pushes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn refuses_a_pending_review_even_when_the_task_was_already_done() {
        let store = approved_project().await;
        let pending = Approval::new(
            ApprovalId::new("review-pending").unwrap(),
            RunId::new("reviewer-1").unwrap(),
            ApprovalKind::Review,
            "{}".into(),
        );
        ApprovalStore::save(&store, &pending).await.unwrap();
        let adapter = FakeDelivery::new(None);
        assert!(use_case(&store, &adapter)
            .execute(&test_fixtures::PROJECT)
            .await
            .is_err());
        assert!(adapter.pushes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unsafe_worktree_leaves_no_delivery_record() {
        let store = approved_project().await;
        let mut adapter = FakeDelivery::new(None);
        adapter.publish_error = true;
        assert!(use_case(&store, &adapter)
            .execute(&test_fixtures::PROJECT)
            .await
            .is_err());
        assert!(
            DeliveryStore::get_for_project(&store, &test_fixtures::PROJECT)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_pr_api_failure_keeps_the_verified_push_evidence() {
        let store = approved_project().await;
        let mut adapter = FakeDelivery::new(None);
        adapter.open_error = true;
        assert!(use_case(&store, &adapter)
            .execute(&test_fixtures::PROJECT)
            .await
            .is_err());
        let delivery = DeliveryStore::get_for_project(&store, &test_fixtures::PROJECT)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivery.status, DeliveryStatus::Pushed);
        assert_eq!(delivery.local_sha, delivery.remote_sha);
        assert!(delivery.pull_request_url.is_none());
    }
}

impl<
        P: ProjectStore,
        T: TaskStore,
        R: RunStore,
        A: ApprovalStore,
        D: DeliveryStore,
        W: WorkBranchPublisher,
        G: GitHubClient,
    > DeliverProject<P, T, R, A, D, W, G>
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        projects: P,
        tasks: T,
        runs: R,
        approvals: A,
        deliveries: D,
        publisher: W,
        github: G,
    ) -> Self {
        Self {
            projects,
            tasks,
            runs,
            approvals,
            deliveries,
            publisher,
            github,
        }
    }

    pub async fn execute(&self, project_id: &ProjectId) -> Result<Delivery, UseCaseError> {
        let project = self
            .projects
            .get(project_id)
            .await?
            .filter(|project| !project.deleted)
            .ok_or(UseCaseError::NotFound("project"))?;
        if project.work_branch == project.default_branch {
            return Err(DomainError::Invariant(
                "delivery work branch must differ from the default branch",
            )
            .into());
        }

        let tasks = self.tasks.list_for_project(project_id).await?;
        if tasks.is_empty() {
            return Err(DomainError::Invariant("delivery needs at least one selected task").into());
        }
        let mut approved_shas = Vec::with_capacity(tasks.len());
        for task in tasks {
            if task.status != TaskStatus::Done {
                return Err(DomainError::Invariant(
                    "every selected task must have an approved review before delivery",
                )
                .into());
            }
            if self.runs.active_for_task(&task.id).await?.is_some() {
                return Err(DomainError::Invariant(
                    "delivery is unavailable while a selected task has an active run",
                )
                .into());
            }

            let runs = self.runs.list_for_task(&task.id).await?;
            let mut granted_reviewer_index = None;
            for (index, run) in runs.iter().enumerate().rev() {
                let decisions = self.approvals.list_for_run(&run.id).await?;
                if decisions
                    .iter()
                    .any(|approval| approval.status == ApprovalStatus::Pending)
                {
                    return Err(DomainError::Invariant(
                        "delivery is unavailable while an approval is pending",
                    )
                    .into());
                }
                if run.role_id.as_str() == "reviewer"
                    && decisions.iter().any(|approval| {
                        approval.kind == ApprovalKind::Review
                            && approval.status == ApprovalStatus::Granted
                    })
                    && granted_reviewer_index.is_none()
                {
                    granted_reviewer_index = Some(index);
                }
            }
            let reviewer_index = granted_reviewer_index.ok_or(DomainError::Invariant(
                "every selected task needs a granted Reviewer decision",
            ))?;
            let executor = runs[..reviewer_index]
                .iter()
                .rev()
                .find(|run| run.role_id.as_str() != "reviewer" && run.status == RunStatus::Finished)
                .ok_or(DomainError::Invariant(
                    "an approved task needs a finished executor run",
                ))?;
            approved_shas.push(executor.head_sha.clone().ok_or(DomainError::Invariant(
                "an approved executor run needs Git SHA evidence",
            ))?);
        }

        approved_shas.sort();
        approved_shas.dedup();
        let published = self
            .publisher
            .verify_and_push(&PublishWorkBranchInput {
                repo: project.github_repo.clone(),
                checkout: project.local_path.clone(),
                work_branch: project.work_branch.clone(),
                approved_shas,
            })
            .await?;
        if published.work_branch != project.work_branch {
            return Err(
                DomainError::Invariant("the publisher returned a different work branch").into(),
            );
        }

        let mut delivery = Delivery::pushed(
            project.id.clone(),
            published.work_branch,
            published.local_sha,
            published.remote_sha,
        )?;
        self.deliveries.save(&delivery).await?;

        let url = match self
            .github
            .find_open_pull_request(
                &project.github_repo,
                &project.work_branch,
                &project.default_branch,
            )
            .await?
        {
            Some(url) => url,
            None => match self
                .github
                .open_pull_request(
                    &project.github_repo,
                    &project.work_branch,
                    &project.default_branch,
                )
                .await
            {
                Ok(url) => url,
                Err(open_error) => {
                    // A concurrent/retried request may have won the create
                    // race. Re-read before surfacing the original failure.
                    match self
                        .github
                        .find_open_pull_request(
                            &project.github_repo,
                            &project.work_branch,
                            &project.default_branch,
                        )
                        .await
                    {
                        Ok(Some(url)) => url,
                        _ => return Err(open_error.into()),
                    }
                }
            },
        };
        delivery.attach_pull_request(url)?;
        self.deliveries.save(&delivery).await?;
        Ok(delivery)
    }
}
