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
        matches!(
            self,
            RunStatus::Starting | RunStatus::Running | RunStatus::Blocked
        )
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
    /// Git evidence captured by the agent adapter. Artifacts are sanitized
    /// JSON (activity, commits, changed files and diff statistics), never
    /// hidden reasoning or raw tool inputs.
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub artifacts: Option<String>,
    /// The executor run this Reviewer run evaluates. Persisting this edge is
    /// what prevents a valid-but-stale evidence id from being replayed on a
    /// different review. It is always `None` for non-Reviewer runs.
    pub reviewed_run_id: Option<RunId>,
}

impl Run {
    pub fn new(id: RunId, task_id: TaskId, role_id: RoleId, triggered_by: TriggeredBy) -> Self {
        Self {
            id,
            task_id,
            role_id,
            triggered_by,
            acp_session_id: None,
            status: RunStatus::Starting,
            summary: None,
            base_sha: None,
            head_sha: None,
            artifacts: None,
            reviewed_run_id: None,
        }
    }

    pub fn bind_review_subject(&mut self, subject: RunId) -> Result<(), DomainError> {
        if self.role_id.as_str() != "reviewer" {
            return Err(DomainError::Invariant(
                "only a Reviewer run can bind a review subject",
            ));
        }
        if subject == self.id {
            return Err(DomainError::Invariant(
                "a Reviewer run cannot review itself",
            ));
        }
        if self
            .reviewed_run_id
            .as_ref()
            .is_some_and(|bound| bound != &subject)
        {
            return Err(DomainError::Invariant(
                "a Reviewer run has one immutable review subject",
            ));
        }
        self.reviewed_run_id = Some(subject);
        Ok(())
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

    pub fn attach_evidence(
        &mut self,
        base_sha: Option<String>,
        head_sha: Option<String>,
        artifacts: String,
    ) -> Result<(), DomainError> {
        if self.status != RunStatus::Finished {
            return Err(DomainError::Invariant(
                "run evidence can only be attached to a finished run",
            ));
        }
        self.base_sha = base_sha;
        self.head_sha = head_sha;
        self.artifacts = Some(artifacts);
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
        r.attach_evidence(Some("base".into()), Some("head".into()), "{}".into())
            .unwrap();
        assert_eq!(r.head_sha.as_deref(), Some("head"));
        assert!(!r.status.is_active());
    }

    #[test]
    fn refused_transitions_are_errors() {
        let mut r = run();
        assert!(r.finish("too early").is_err()); // Starting → Finished
        assert!(r.attach_evidence(None, None, "{}".into()).is_err());
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

    #[test]
    fn only_a_reviewer_can_bind_one_immutable_subject() {
        let mut executor = run();
        assert!(executor
            .bind_review_subject(RunId::new("subject").unwrap())
            .is_err());

        let mut reviewer = Run::new(
            RunId::new("reviewer").unwrap(),
            TaskId::new("t1").unwrap(),
            RoleId::new("reviewer").unwrap(),
            TriggeredBy::Manager,
        );
        reviewer
            .bind_review_subject(RunId::new("subject").unwrap())
            .unwrap();
        reviewer
            .bind_review_subject(RunId::new("subject").unwrap())
            .unwrap();
        assert!(reviewer
            .bind_review_subject(RunId::new("other").unwrap())
            .is_err());
        assert!(reviewer.bind_review_subject(reviewer.id.clone()).is_err());
    }
}
