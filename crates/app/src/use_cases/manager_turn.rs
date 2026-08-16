//! `ManagerTurn` — the Manager answered; make it real. Parses the actions
//! block out of the reply (manager_actions.rs), executes each action through
//! the existing use cases and ports, and persists the reply as a thread
//! message whose `actions` are the display cards the UI renders.
//!
//! Refusals are content, not failures: a dispatch without an approved spec
//! becomes a "Dispatch refused" card in the thread — the owner sees exactly
//! what the Manager tried and why it didn't happen. Real adapter failures
//! (store down, channel dead) still propagate as errors.
//!
//! Takes the concrete `Store` (like CreateProject): the ports expose no
//! spec-list read, which ProposeSpec needs to number versions.

use super::{DispatchTask, DispatchTaskInput, UseCaseError};
use crate::manager_actions::{parse_reply, ManagerAction};
use crate::store::Store;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{MessageId, ProjectId, RoleId, SpecVersionId, TaskId};
use latoile_core::ports::{AgentChannel, ConversationStore, EventLog, ManagerReply, SpecStore, TaskStore};
use latoile_core::{Author, Message, SpecVersion, Task, TriggeredBy};

pub struct ManagerTurn<A> {
    store: Store,
    agents: A,
}

/// The reply as persisted, plus the parse warnings (already surfaced as
/// cards — this is for the caller's logs).
pub struct ManagerOutcome {
    pub message: Message,
    pub warnings: Vec<String>,
}

impl<A: AgentChannel + Clone> ManagerTurn<A> {
    pub fn new(store: Store, agents: A) -> Self {
        Self { store, agents }
    }

    pub async fn record_reply(
        &self,
        project: &ProjectId,
        reply: ManagerReply,
    ) -> Result<ManagerOutcome, UseCaseError> {
        let parsed = parse_reply(&reply.content);

        let mut cards: Vec<serde_json::Value> = Vec::new();
        for warning in &parsed.warnings {
            cards.push(serde_json::json!({"title": format!("⚠ {warning}")}));
        }
        for action in &parsed.actions {
            self.execute(project, action, &mut cards).await?;
        }

        // The reply, block stripped; cards become the message's actions.
        let content = if parsed.display_text.is_empty() {
            "(actions only)".to_string()
        } else {
            parsed.display_text
        };
        let conversation = self
            .store
            .for_project(project)
            .await?
            .ok_or(UseCaseError::NotFound("conversation"))?;
        let actions = if cards.is_empty() {
            reply.actions // the channel supplied none today, but don't drop one
        } else {
            Some(serde_json::to_string(&cards).expect("cards serialize"))
        };
        let message = Message::new(
            MessageId::new(ulid::Ulid::new().to_string())?,
            conversation.id,
            Author::Manager,
            content,
            actions,
        )?;
        ConversationStore::append(&self.store, &message).await?;
        EventLog::append(
            &self.store,
            &NewEvent {
                project_id: project.clone(),
                kind: EventKind::MessagePosted,
                payload: format!("{{\"message_id\":\"{}\"}}", message.id),
            },
        )
        .await?;

        Ok(ManagerOutcome {
            message,
            warnings: parsed.warnings,
        })
    }

    async fn execute(
        &self,
        project: &ProjectId,
        action: &ManagerAction,
        cards: &mut Vec<serde_json::Value>,
    ) -> Result<(), UseCaseError> {
        match action {
            ManagerAction::CreateTasks { tasks } => {
                let mut position = self.store.list_for_project(project).await?.len() as u32;
                for new in tasks {
                    let task = Task::new(
                        TaskId::new(ulid::Ulid::new().to_string())?,
                        project.clone(),
                        RoleId::new(&new.role_id)?,
                        &new.title,
                        &new.description,
                        position,
                    )?;
                    position += 1;
                    // A role the Manager invented fails the FK — a card, not
                    // a crash; the rest of the batch still lands.
                    if let Err(e) = TaskStore::save(&self.store, &task).await {
                        cards.push(serde_json::json!({
                            "title": format!("⚠ Task refused: {}", new.title),
                            "sub": e.to_string(),
                        }));
                        continue;
                    }
                    EventLog::append(
                        &self.store,
                        &NewEvent {
                            project_id: project.clone(),
                            kind: EventKind::TaskReady,
                            payload: format!("{{\"task_id\":\"{}\"}}", task.id.as_str()),
                        },
                    )
                    .await?;
                    cards.push(serde_json::json!({
                        "title": format!("Task created: {} → {}", new.title, new.role_id),
                        "sub": "ready column",
                    }));
                }
                Ok(())
            }
            ManagerAction::DispatchTask {
                title,
                role_id,
                prompt,
            } => {
                let position = self.store.list_for_project(project).await?.len() as u32;
                let result = DispatchTask::new(
                    self.store.clone(),
                    self.store.clone(),
                    self.store.clone(),
                    self.agents.clone(),
                    self.store.clone(),
                )
                .execute(DispatchTaskInput {
                    project_id: project.clone(),
                    role_id: RoleId::new(role_id)?,
                    title: title.clone(),
                    description: String::new(),
                    position,
                    triggered_by: TriggeredBy::Manager,
                    prompt: prompt.clone(),
                })
                .await;
                match result {
                    Ok(dispatched) => cards.push(serde_json::json!({
                        "title": format!("Run started — {title}"),
                        "sub": format!("run {}", dispatched.run.id.as_str()),
                    })),
                    // Spec-before-code and friends: visible, not fatal.
                    Err(UseCaseError::Domain(e)) => cards.push(serde_json::json!({
                        "title": format!("Dispatch refused: {title}"),
                        "sub": e.to_string(),
                    })),
                    Err(e) => return Err(e),
                }
                Ok(())
            }
            ManagerAction::ProposeSpec { design_dir } => {
                let version = self.store.specs_for_project(project).await?.len() as u32 + 1;
                let spec = SpecVersion::new(
                    SpecVersionId::new(ulid::Ulid::new().to_string())?,
                    project.clone(),
                    version,
                    design_dir,
                    None,
                )?;
                SpecStore::save(&self.store, &spec).await?;
                EventLog::append(
                    &self.store,
                    &NewEvent {
                        project_id: project.clone(),
                        kind: EventKind::SpecVersionCreated,
                        payload: format!("{{\"spec_version_id\":\"{}\"}}", spec.id.as_str()),
                    },
                )
                .await?;
                cards.push(serde_json::json!({
                    "title": format!("Spec v{version} drafted"),
                    "sub": format!("{design_dir} · awaiting your approval"),
                }));
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ids::RunId;
    use latoile_core::ports::PortResult;
    use latoile_core::Run;

    #[derive(Clone)]
    struct FakeAgents;

    impl AgentChannel for FakeAgents {
        async fn tell_manager(&self, _p: &ProjectId, _m: &str) -> PortResult<ManagerReply> {
            unimplemented!()
        }
        async fn start_run(&self, _r: &Run, _p: &str) -> PortResult<String> {
            Ok("acp-fake".into())
        }
        async fn cancel_run(&self, _r: &RunId) -> PortResult<()> {
            Ok(())
        }
    }

    async fn store_with_thread() -> Store {
        let store = test_fixtures::store_with_project().await;
        store
            .create_conversation(&latoile_core::conversation::Conversation::new(
                latoile_core::ids::ConversationId::new("c1").unwrap(),
                test_fixtures::PROJECT.clone(),
            ))
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn actions_execute_and_the_reply_keeps_only_prose_and_cards() {
        let store = store_with_thread().await;
        let reply = ManagerReply {
            content: "Je m'en occupe.\n\n```latoile-actions\n[{\"type\": \"create_tasks\", \"tasks\": [{\"title\": \"Login page\", \"role_id\": \"frontend\", \"description\": \"Form\"}]}]\n```".into(),
            actions: None,
        };
        let outcome = ManagerTurn::new(store.clone(), FakeAgents)
            .record_reply(&test_fixtures::PROJECT, reply)
            .await
            .unwrap();

        assert_eq!(outcome.message.content, "Je m'en occupe.");
        let cards: serde_json::Value =
            serde_json::from_str(outcome.message.actions.as_deref().unwrap()).unwrap();
        assert_eq!(cards[0]["title"], "Task created: Login page → frontend");

        let tasks = store.list_for_project(&test_fixtures::PROJECT).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Login page");
        let events = store.events_since(0).await.unwrap();
        assert!(events.iter().any(|(_, e)| e.kind == EventKind::TaskReady));
        assert!(events.iter().any(|(_, e)| e.kind == EventKind::MessagePosted));
    }

    #[tokio::test]
    async fn dispatch_without_a_spec_is_a_card_not_a_crash() {
        let store = store_with_thread().await; // no approved spec
        let reply = ManagerReply {
            content: "```latoile-actions\n[{\"type\": \"dispatch_task\", \"title\": \"Login page\", \"role_id\": \"frontend\", \"prompt\": \"Build it\"}]\n```".into(),
            actions: None,
        };
        let outcome = ManagerTurn::new(store.clone(), FakeAgents)
            .record_reply(&test_fixtures::PROJECT, reply)
            .await
            .unwrap();

        let cards: serde_json::Value =
            serde_json::from_str(outcome.message.actions.as_deref().unwrap()).unwrap();
        assert_eq!(cards[0]["title"], "Dispatch refused: Login page");
        assert!(store
            .list_for_project(&test_fixtures::PROJECT)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn dispatch_with_a_spec_starts_a_run() {
        let store = test_fixtures::store_with_approved_spec().await;
        store
            .create_conversation(&latoile_core::conversation::Conversation::new(
                latoile_core::ids::ConversationId::new("c1").unwrap(),
                test_fixtures::PROJECT.clone(),
            ))
            .await
            .unwrap();
        let reply = ManagerReply {
            content: "Go.\n```latoile-actions\n[{\"type\": \"dispatch_task\", \"title\": \"Login page\", \"role_id\": \"frontend\", \"prompt\": \"Build it\"}]\n```".into(),
            actions: None,
        };
        let outcome = ManagerTurn::new(store.clone(), FakeAgents)
            .record_reply(&test_fixtures::PROJECT, reply)
            .await
            .unwrap();

        let cards: serde_json::Value =
            serde_json::from_str(outcome.message.actions.as_deref().unwrap()).unwrap();
        assert_eq!(cards[0]["title"], "Run started — Login page");
        let tasks = store.list_for_project(&test_fixtures::PROJECT).await.unwrap();
        assert_eq!(tasks[0].status, latoile_core::TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn propose_spec_numbers_the_next_draft() {
        let store = test_fixtures::store_with_approved_spec().await; // v1 exists
        store
            .create_conversation(&latoile_core::conversation::Conversation::new(
                latoile_core::ids::ConversationId::new("c1").unwrap(),
                test_fixtures::PROJECT.clone(),
            ))
            .await
            .unwrap();
        let reply = ManagerReply {
            content: "```latoile-actions\n[{\"type\": \"propose_spec\"}]\n```".into(),
            actions: None,
        };
        let outcome = ManagerTurn::new(store.clone(), FakeAgents)
            .record_reply(&test_fixtures::PROJECT, reply)
            .await
            .unwrap();

        let specs = store.specs_for_project(&test_fixtures::PROJECT).await.unwrap();
        assert_eq!(specs[0].version, 2);
        assert_eq!(specs[0].status, latoile_core::SpecStatus::Draft);
        assert_eq!(outcome.warnings.len(), 0);
    }

    #[tokio::test]
    async fn malformed_blocks_become_warning_cards() {
        let store = store_with_thread().await;
        let reply = ManagerReply {
            content: "Réponse.\n```latoile-actions\n[oops]\n```".into(),
            actions: None,
        };
        let outcome = ManagerTurn::new(store.clone(), FakeAgents)
            .record_reply(&test_fixtures::PROJECT, reply)
            .await
            .unwrap();

        assert_eq!(outcome.warnings.len(), 1);
        let cards: serde_json::Value =
            serde_json::from_str(outcome.message.actions.as_deref().unwrap()).unwrap();
        assert!(cards[0]["title"].as_str().unwrap().starts_with('⚠'));
    }
}
