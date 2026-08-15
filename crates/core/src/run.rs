//! `Run` — one agent execution on a task. Ephemeral: born for a task, dies
//! at the end. The Manager's persistent session is a different lifecycle and
//! lives in the agents adapter, not here.

use crate::error::{DomainError, TransitionError};
use crate::ids::{RoleId, RunId, TaskId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Starting,
    Running,
    Blocked,
    Finished,
    Error,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Starting => "starting",
            RunStatus::Running => "running",
            RunStatus::Blocked => "blocked",
            RunStatus::Finished => "finished",
            RunStatus::Error => "error",
            RunStatus::Cancelled => "cancelled",
        }
    }

    /// An active run holds the task's single-run slot (invariant §3.2.1).
    pub fn is_active(&self) -> bool {
        matches!(self, RunStatus::Starting | RunStatus::Running | RunStatus::Blocked)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggeredBy {
    User,
    Manager,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: RunId,
    pub task_id: TaskId,
    pub role_id: RoleId,
    pub triggered_by: TriggeredBy,
    pub acp_session_id: Option<String>,
    pub status: RunStatus,
    pub summary: Option<String>,
}

impl Run {
    pub fn new(
        id: RunId,
        task_id: TaskId,
        role_id: RoleId,
        triggered_by: TriggeredBy,
    ) -> Self {
        Self {
            id,
            task_id,
            role_id,
            triggered_by,
            acp_session_id: None,
            status: RunStatus::Starting,
            summary: None,
        }
    }

    fn go(&mut self, to: RunStatus) -> Result<(), DomainError> {
        let allowed = matches!(
            (self.status, to),
            (RunStatus::Starting, RunStatus::Running)
                | (RunStatus::Running, RunStatus::Blocked)
                | (RunStatus::Blocked, RunStatus::Running)
                | (RunStatus::Running, RunStatus::Finished)
                | (RunStatus::Starting, RunStatus::Error)
                | (RunStatus::Running, RunStatus::Error)
                | (RunStatus::Blocked, RunStatus::Error)
                | (RunStatus::Starting, RunStatus::Cancelled)
                | (RunStatus::Running, RunStatus::Cancelled)
                | (RunStatus::Blocked, RunStatus::Cancelled)
        );
        if !allowed {
            return Err(TransitionError::new("run", self.status.as_str(), to.as_str()).into());
        }
        self.status = to;
        Ok(())
    }

    /// Handshake done, the agent is working.
    pub fn begin(&mut self) -> Result<(), DomainError> {
        self.go(RunStatus::Running)
    }

    /// The agent needs a permission or an answer — it is parked, not dead.
    pub fn block(&mut self) -> Result<(), DomainError> {
        self.go(RunStatus::Blocked)
    }

    pub fn resume(&mut self) -> Result<(), DomainError> {
        self.go(RunStatus::Running)
    }

    pub fn finish(&mut self, summary: impl Into<String>) -> Result<(), DomainError> {
        self.go(RunStatus::Finished)?;
        self.summary = Some(summary.into());
        Ok(())
    }

    pub fn fail(&mut self) -> Result<(), DomainError> {
        self.go(RunStatus::Error)
    }

    pub fn cancel(&mut self) -> Result<(), DomainError> {
        self.go(RunStatus::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> Run {
        Run::new(
            RunId::new("r1").unwrap(),
            TaskId::new("t1").unwrap(),
            RoleId::new("backend").unwrap(),
            TriggeredBy::Manager,
        )
    }

    #[test]
    fn happy_path_start_block_resume_finish() {
        let mut r = run();
        r.begin().unwrap();
        r.block().unwrap();
        assert!(r.status.is_active());
        r.resume().unwrap();
        r.finish("done: endpoint implemented").unwrap();
        assert_eq!(r.status, RunStatus::Finished);
        assert_eq!(r.summary.as_deref(), Some("done: endpoint implemented"));
        assert!(!r.status.is_active());
    }

    #[test]
    fn refused_transitions_are_errors() {
        let mut r = run();
        assert!(r.finish("too early").is_err()); // Starting → Finished
        assert!(r.block().is_err()); // Starting → Blocked
        r.begin().unwrap();
        r.finish("ok").unwrap();
        assert!(r.cancel().is_err()); // Finished is terminal
        assert!(r.resume().is_err());
    }

    #[test]
    fn cancel_works_from_every_active_state() {
        for setup in [0, 1, 2] {
            let mut r = run();
            if setup >= 1 {
                r.begin().unwrap();
            }
            if setup >= 2 {
                r.block().unwrap();
            }
            r.cancel().unwrap();
            assert_eq!(r.status, RunStatus::Cancelled);
        }
    }

    #[test]
    fn error_is_terminal() {
        let mut r = run();
        r.fail().unwrap(); // Starting → Error
        assert!(r.begin().is_err());
    }
}
