//! Domain errors. Deliberately small: invalid state transitions and malformed
//! values. Anything I/O-shaped belongs to the adapters, not here.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// An id was built from an empty or blank string.
    EmptyId(&'static str),
    /// A state machine refused a transition.
    Transition(TransitionError),
    /// A business invariant was violated (e.g. approving with the wrong
    /// approval kind).
    Invariant(&'static str),
}

/// A refused state-machine transition, with the states involved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionError {
    pub entity: &'static str,
    pub from: String,
    pub to: &'static str,
}

impl TransitionError {
    pub fn new(entity: &'static str, from: impl Into<String>, to: &'static str) -> Self {
        Self {
            entity,
            from: from.into(),
            to,
        }
    }
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} cannot go from {} to {}",
            self.entity, self.from, self.to
        )
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DomainError::EmptyId(kind) => write!(f, "{kind} cannot be empty"),
            DomainError::Transition(t) => t.fmt(f),
            DomainError::Invariant(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for DomainError {}

impl From<TransitionError> for DomainError {
    fn from(t: TransitionError) -> Self {
        DomainError::Transition(t)
    }
}
