//! `conversation` and `message` tables — the permanent Manager thread.
//! Exactly one conversation per project (UNIQUE on `project_id`).

use super::{unknown_variant, Store, StoreError};
use latoile_core::ports::{ConversationStore, PortResult};
use latoile_core::{Author, Conversation, ConversationId, Message, MessageId, ProjectId};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

fn parse_author(raw: &str) -> Result<Author, StoreError> {
    Ok(match raw {
        "user" => Author::User,
        "manager" => Author::Manager,
        other => return Err(unknown_variant("message author", other)),
    })
}

fn row_to_message(row: &SqliteRow) -> Result<Message, StoreError> {
    Ok(Message {
        id: MessageId::new(row.try_get::<String, _>("id")?)?,
        conversation_id: ConversationId::new(row.try_get::<String, _>("conversation_id")?)?,
        author: parse_author(&row.try_get::<String, _>("author")?)?,
        content: row.try_get("content")?,
        actions: row.try_get("actions")?,
    })
}

const COLUMNS: &str = "id, conversation_id, author, content, actions";

impl Store {
    /// The conversation is created with the project (onboarding); it is not
    /// part of the port because no use case ever needs a project without one.
    pub async fn create_conversation(&self, conversation: &Conversation) -> PortResult<()> {
        sqlx::query("INSERT INTO conversation (id, project_id) VALUES (?, ?)")
            .bind(conversation.id.as_str())
            .bind(conversation.project_id.as_str())
            .execute(self.pool())
            .await
            .map_err(StoreError::from)?;
        Ok(())
    }
}

impl ConversationStore for Store {
    async fn for_project(&self, project: &ProjectId) -> PortResult<Option<Conversation>> {
        let row = sqlx::query("SELECT id, project_id FROM conversation WHERE project_id = ?")
            .bind(project.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(StoreError::from)?;
        row.map(
            |r| -> Result<Conversation, StoreError> {
                Ok(Conversation {
                    id: ConversationId::new(r.try_get::<String, _>("id")?)?,
                    project_id: ProjectId::new(r.try_get::<String, _>("project_id")?)?,
                })
            },
        )
        .transpose()
        .map_err(Into::into)
    }

    async fn append(&self, message: &Message) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO message (id, conversation_id, author, content, actions)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(message.id.as_str())
        .bind(message.conversation_id.as_str())
        .bind(message.author.as_str())
        .bind(&message.content)
        .bind(&message.actions)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    /// The `limit` most recent messages, oldest first — chat order.
    async fn recent(&self, conversation: &ProjectId, limit: u32) -> PortResult<Vec<Message>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM message
             WHERE conversation_id = (SELECT id FROM conversation WHERE project_id = ?)
             ORDER BY created_at DESC, rowid DESC LIMIT ?"
        ))
        .bind(conversation.as_str())
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        let mut messages: Vec<Message> = rows
            .iter()
            .map(row_to_message)
            .collect::<Result<Vec<_>, _>>()?;
        messages.reverse();
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;

    async fn store_with_conversation() -> (Store, ConversationId) {
        let s = test_fixtures::store_with_project().await;
        let c = Conversation::new(
            ConversationId::new("c1").unwrap(),
            test_fixtures::PROJECT.clone(),
        );
        s.create_conversation(&c).await.unwrap();
        (s, c.id)
    }

    fn message(id: &str, conversation: &ConversationId, author: Author, content: &str) -> Message {
        Message::new(
            MessageId::new(id).unwrap(),
            conversation.clone(),
            author,
            content,
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn the_thread_round_trips() {
        let (s, conv) = store_with_conversation().await;
        let m = message("m1", &conv, Author::Manager, "Voici le plan.");
        s.append(&m).await.unwrap();

        let back = s
            .recent(&test_fixtures::PROJECT, 10)
            .await
            .unwrap();
        assert_eq!(back, vec![m]);
    }

    #[tokio::test]
    async fn recent_returns_the_last_n_oldest_first() {
        let (s, conv) = store_with_conversation().await;
        for (id, content) in [("m1", "un"), ("m2", "deux"), ("m3", "trois")] {
            s.append(&message(id, &conv, Author::User, content))
                .await
                .unwrap();
        }

        let last_two = s.recent(&test_fixtures::PROJECT, 2).await.unwrap();
        assert_eq!(last_two.len(), 2);
        assert_eq!(last_two[0].content, "deux");
        assert_eq!(last_two[1].content, "trois");
    }

    #[tokio::test]
    async fn one_conversation_per_project() {
        let (s, _) = store_with_conversation().await;
        let dup = Conversation::new(
            ConversationId::new("c2").unwrap(),
            test_fixtures::PROJECT.clone(),
        );
        assert!(s.create_conversation(&dup).await.is_err());
    }

    #[tokio::test]
    async fn for_project_finds_the_conversation() {
        let (s, conv) = store_with_conversation().await;
        let found = s.for_project(&test_fixtures::PROJECT).await.unwrap().unwrap();
        assert_eq!(found.id, conv);
    }
}
