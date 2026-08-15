//! `SendMessage` — the owner posts on the project's Manager thread. The
//! message is persisted and journaled; the Manager's reply is a separate
//! concern of the agents adapter.

use super::UseCaseError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{MessageId, ProjectId};
use latoile_core::ports::{ConversationStore, EventLog};
use latoile_core::{Author, Message};

pub struct SendMessageInput {
    pub project_id: ProjectId,
    pub content: String,
}

/// What the UI needs to render the thread without a refetch.
pub struct PostedMessage {
    pub message: Message,
    pub seq: u64,
}

pub struct SendMessage<C, E> {
    conversations: C,
    events: E,
}

impl<C: ConversationStore, E: EventLog> SendMessage<C, E> {
    pub fn new(conversations: C, events: E) -> Self {
        Self {
            conversations,
            events,
        }
    }

    pub async fn execute(&self, input: SendMessageInput) -> Result<PostedMessage, UseCaseError> {
        // 2. Fetch: the conversation is created with the project.
        let conversation = self
            .conversations
            .for_project(&input.project_id)
            .await?
            .ok_or(UseCaseError::NotFound("conversation"))?;

        // 3. Domain: `Message::new` refuses blank content (1. validation).
        let message = Message::new(
            MessageId::new(ulid::Ulid::new().to_string())?,
            conversation.id.clone(),
            Author::User,
            input.content,
            None,
        )?;

        // 4. Persist.
        self.conversations.append(&message).await?;

        // 5. Journal. The payload is built by hand: a ulid is bare ASCII,
        // so there is nothing to escape.
        let seq = self
            .events
            .append(&NewEvent {
                project_id: input.project_id,
                kind: EventKind::MessagePosted,
                payload: format!("{{\"message_id\":\"{}\"}}", message.id),
            })
            .await?;

        // 6. DTO.
        Ok(PostedMessage { message, seq })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use crate::store::Store;
    use latoile_core::conversation::Conversation;
    use latoile_core::ids::ConversationId;

    async fn setup() -> SendMessage<Store, Store> {
        let store = test_fixtures::store_with_project().await;
        store
            .create_conversation(&Conversation::new(
                ConversationId::new("c1").unwrap(),
                test_fixtures::PROJECT.clone(),
            ))
            .await
            .unwrap();
        SendMessage::new(store.clone(), store)
    }

    #[tokio::test]
    async fn a_message_is_persisted_and_journaled() {
        let uc = setup().await;
        let posted = uc
            .execute(SendMessageInput {
                project_id: test_fixtures::PROJECT.clone(),
                content: "Construis la page de connexion".into(),
            })
            .await
            .unwrap();

        assert_eq!(posted.message.author, Author::User);
        let events = uc
            .events
            .since(&test_fixtures::PROJECT, 0)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, posted.seq);
        assert_eq!(events[0].1.kind, EventKind::MessagePosted);
    }

    #[tokio::test]
    async fn a_blank_message_is_refused_and_nothing_is_persisted() {
        let uc = setup().await;
        assert!(uc
            .execute(SendMessageInput {
                project_id: test_fixtures::PROJECT.clone(),
                content: "   ".into(),
            })
            .await
            .is_err());
        assert!(uc
            .events
            .since(&test_fixtures::PROJECT, 0)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn an_unknown_project_is_refused() {
        let uc = setup().await;
        assert!(uc
            .execute(SendMessageInput {
                project_id: ProjectId::new("ghost").unwrap(),
                content: "allo".into(),
            })
            .await
            .is_err());
    }
}
