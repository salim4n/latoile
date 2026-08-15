//! `Conversation` and `Message` — the permanent thread with the project's
//! Manager. One conversation per project. Messages from the Manager carry a
//! structured action list so the thread doubles as an intentions journal.

use crate::error::DomainError;
use crate::ids::{ConversationId, MessageId, ProjectId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub id: ConversationId,
    pub project_id: ProjectId,
}

impl Conversation {
    pub fn new(id: ConversationId, project_id: ProjectId) -> Self {
        Self { id, project_id }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author {
    User,
    Manager,
}

impl Author {
    pub fn as_str(&self) -> &'static str {
        match self {
            Author::User => "user",
            Author::Manager => "manager",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    pub author: Author,
    pub content: String,
    /// JSON action list on Manager messages: tasks created, runs started,
    /// approvals requested — each with its reference.
    pub actions: Option<String>,
}

impl Message {
    pub fn new(
        id: MessageId,
        conversation_id: ConversationId,
        author: Author,
        content: impl Into<String>,
        actions: Option<String>,
    ) -> Result<Self, DomainError> {
        let content = content.into();
        if content.trim().is_empty() {
            return Err(DomainError::Invariant("a message needs content"));
        }
        Ok(Self {
            id,
            conversation_id,
            author,
            content,
            actions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_messages_are_refused() {
        assert!(Message::new(
            MessageId::new("m1").unwrap(),
            ConversationId::new("c1").unwrap(),
            Author::User,
            "   ",
            None,
        )
        .is_err());
    }
}
