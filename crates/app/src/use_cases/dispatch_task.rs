//! `DispatchTask` — queue a task and start its first run. Enforces the
//! spec-before-code rule: without an approved spec on the project,
//! `Task::start()` refuses and nothing is persisted.

use super::UseCaseError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{ProjectId, RoleId, RunId, TaskId};
use latoile_core::ports::{AgentChannel, EventLog, RunStore, SpecStore, TaskStore};
use latoile_core::{DomainError, Run, Task, TriggeredBy};

pub struct DispatchTaskInput {
    pub project_id: ProjectId,
    pub role_id: RoleId,
    pub title: String,
    pub description: String,
    pub position: u32,
    pub triggered_by: TriggeredBy,
    /// What the executor agent is told — typically task + spec references.
    pub prompt: String,
}

pub struct DispatchedTask {
    pub task: Task,
    pub run: Run,
}

pub struct DispatchTask<S, T, R, A, E> {
    specs: S,
    tasks: T,
    runs: R,
    agents: A,
    events: E,
}

impl<S: SpecStore, T: TaskStore, R: RunStore, A: AgentChannel, E: EventLog>
    DispatchTask<S, T, R, A, E>
{
    pub fn new(specs: S, tasks: T, runs: R, agents: A, events: E) -> Self {
        Self {
            specs,
            tasks,
            runs,
            agents,
            events,
        }
    }

    pub async fn execute(&self, input: DispatchTaskInput) -> Result<DispatchedTask, UseCaseError> {
        // 2. Fetch: the approved spec this task materializes, if any.
        let spec = self.specs.approved_for_project(&input.project_id).await?;
        if let Some(spec) = &spec {
            let verification = self
                .agents
                .verify_architecture_package(&input.project_id, spec)
                .await?;
            if !verification.valid {
                return Err(DomainError::Invariant(
                    "the approved architecture package changed; create and approve a new version before dispatch",
                )
                .into());
            }
        }

        // 3. Domain. `Task::new` validates the title; `start` refuses
        // without a spec — before anything hits the database.
        let mut task = Task::new(
            TaskId::new(ulid::Ulid::new().to_string())?,
            input.project_id.clone(),
            input.role_id,
            input.title,
            input.description,
            input.position,
        )?;
        if let Some(spec) = &spec {
            task.bind_spec(spec.id.clone());
        }
        task.start()?;

        let mut run = Run::new(
            RunId::new(ulid::Ulid::new().to_string())?,
            task.id.clone(),
            task.role_id.clone(),
            input.triggered_by,
        );

        // The agent channel is the only way a process ever starts.
        let session = self
            .agents
            .start_run(&input.project_id, &run, &input.prompt)
            .await?;
        run.acp_session_id = Some(session);
        run.begin()?;

        // 4. Persist.
        self.tasks.save(&task).await?;
        self.runs.save(&run).await?;

        // 5. Journal.
        self.events
            .append(&NewEvent {
                project_id: input.project_id.clone(),
                kind: EventKind::TaskReady,
                payload: format!("{{\"task_id\":\"{}\"}}", task.id),
            })
            .await?;
        self.events
            .append(&NewEvent {
                project_id: input.project_id,
                kind: EventKind::RunStarted,
                payload: format!("{{\"task_id\":\"{}\",\"run_id\":\"{}\"}}", task.id, run.id),
            })
            .await?;

        // 6. DTO.
        Ok(DispatchedTask { task, run })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ports::{ManagerReply, PortResult};
    use latoile_core::{RunStatus, TaskStatus};

    /// A fake agent channel: never spawns, hands out session handles.
    struct FakeAgents;

    impl AgentChannel for FakeAgents {
        async fn tell_manager(&self, _p: &ProjectId, _m: &str) -> PortResult<ManagerReply> {
            unimplemented!()
        }
        async fn start_run(
            &self,
            _project: &ProjectId,
            _r: &Run,
            _prompt: &str,
        ) -> PortResult<String> {
            Ok("acp-session-1".into())
        }
        async fn verify_architecture_package(
            &self,
            _project: &ProjectId,
            spec: &latoile_core::SpecVersion,
        ) -> PortResult<latoile_core::ArchitecturePackageValidation> {
            Ok(test_fixtures::test_verification(spec))
        }
        async fn cancel_run(&self, _r: &RunId) -> PortResult<()> {
            Ok(())
        }
    }

    fn input(project: ProjectId) -> DispatchTaskInput {
        DispatchTaskInput {
            project_id: project,
            role_id: RoleId::new("frontend").unwrap(),
            title: "Page de connexion".into(),
            description: "Formulaire email + mot de passe".into(),
            position: 0,
            triggered_by: TriggeredBy::Manager,
            prompt: "Implémente la page de connexion selon design/".into(),
        }
    }

    #[tokio::test]
    async fn a_task_is_dispatched_with_a_run_and_two_events() {
        let store = test_fixtures::store_with_approved_spec().await;
        let uc = DispatchTask::new(
            store.clone(),
            store.clone(),
            store.clone(),
            FakeAgents,
            store.clone(),
        );

        let out = uc
            .execute(input(test_fixtures::PROJECT.clone()))
            .await
            .unwrap();

        assert_eq!(out.task.status, TaskStatus::InProgress);
        assert_eq!(
            out.task.spec_version_id.as_ref().map(|s| s.as_str()),
            Some(test_fixtures::SPEC)
        );
        assert_eq!(out.run.status, RunStatus::Running);
        assert_eq!(out.run.acp_session_id.as_deref(), Some("acp-session-1"));

        // Round-trip through the store (`get` exists on several traits —
        // name the one we mean).
        let persisted = TaskStore::get(&store, &out.task.id).await.unwrap().unwrap();
        assert_eq!(persisted, out.task);

        let events = store.since(&test_fixtures::PROJECT, 0).await.unwrap();
        assert_eq!(
            events.iter().map(|(_, e)| e.kind).collect::<Vec<_>>(),
            [EventKind::TaskReady, EventKind::RunStarted]
        );
    }

    #[tokio::test]
    async fn spec_before_code_is_enforced_and_nothing_is_persisted() {
        // A project with no approved spec.
        let store = test_fixtures::store_with_project().await;
        let uc = DispatchTask::new(
            store.clone(),
            store.clone(),
            store.clone(),
            FakeAgents,
            store.clone(),
        );

        assert!(
            uc.execute(input(test_fixtures::PROJECT.clone()))
                .await
                .is_err()
        );
        assert!(
            store
                .list_for_project(&test_fixtures::PROJECT)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .since(&test_fixtures::PROJECT, 0)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
