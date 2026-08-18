//! Typed identifiers. Newtypes around `String` so a `TaskId` can never be
//! passed where a `RunId` is expected. Generation is the application layer's
//! job (it owns the id generator port); the domain only wraps and validates.

/// Declare an id newtype: non-empty string wrapper with Display.
macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Result<Self, crate::error::DomainError> {
                let id = id.into();
                if id.trim().is_empty() {
                    return Err(crate::error::DomainError::EmptyId(stringify!($name)));
                }
                Ok(Self(id))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_newtype!(ProjectId);
id_newtype!(SpecVersionId);
id_newtype!(TaskId);
id_newtype!(RunId);
id_newtype!(ApprovalId);
id_newtype!(PreviewId);
id_newtype!(ConversationId);
id_newtype!(MessageId);
id_newtype!(ArchitectureSessionId);
id_newtype!(ArchitectureQuestionId);
id_newtype!(VisualComparisonId);

id_newtype!(
    /// A role identifier (`manager`, `architect`, `backend`, `frontend`,
    /// `reviewer`, …). Roles live in the database so new ones can be added
    /// without recompiling; the domain treats the id as opaque but non-empty.
    RoleId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_reject_empty_and_blank_strings() {
        assert!(ProjectId::new("").is_err());
        assert!(ProjectId::new("   ").is_err());
        assert!(ProjectId::new("01J…").is_ok());
    }

    #[test]
    fn ids_of_different_kinds_do_not_mix() {
        let task = TaskId::new("t1").unwrap();
        let run = RunId::new("t1").unwrap();
        // Same payload, different types: this function only accepts TaskId.
        fn takes_task(_: &TaskId) {}
        takes_task(&task);
        let _ = run; // would not compile if passed above — that is the point
    }
}
