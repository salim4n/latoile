//! `Task` — the unit of work on the board. Its state machine enforces the two
//! rules that make LaToile what it is: a task builds only against an approved
//! spec, and it reaches `done` only through a granted review approval.

use crate::approval::{Approval, ApprovalKind};
use crate::error::{DomainError, TransitionError};
use crate::ids::{ProjectId, RoleId, SpecVersionId, TaskId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Ready,
    InProgress,
    Review,
    ChangesRequested,
    Done,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Ready => "ready",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Review => "review",
            TaskStatus::ChangesRequested => "changes_requested",
            TaskStatus::Done => "done",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    /// None while the task sits in `ready`; required to start (spec before
    /// code — decision D7).
    pub spec_version_id: Option<SpecVersionId>,
    pub role_id: RoleId,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub position: u32,
}

impl Task {
    pub fn new(
        id: TaskId,
        project_id: ProjectId,
        role_id: RoleId,
        title: impl Into<String>,
        description: impl Into<String>,
        position: u32,
    ) -> Result<Self, DomainError> {
        let title = title.into();
        if title.trim().is_empty() {
            return Err(DomainError::Invariant("a task needs a title"));
        }
        Ok(Self {
            id,
            project_id,
            spec_version_id: None,
            role_id,
            title,
            description: description.into(),
            status: TaskStatus::Ready,
            position,
        })
    }

    /// Attach the approved spec this task materializes. Required before start.
    pub fn bind_spec(&mut self, spec: SpecVersionId) {
        self.spec_version_id = Some(spec);
    }

    fn go(&mut self, to: TaskStatus) -> Result<(), DomainError> {
        let allowed = matches!(
            (self.status, to),
            (TaskStatus::Ready, TaskStatus::InProgress)
                | (TaskStatus::InProgress, TaskStatus::Review)
                | (TaskStatus::Review, TaskStatus::ChangesRequested)
                | (TaskStatus::ChangesRequested, TaskStatus::Ready)
                | (TaskStatus::Review, TaskStatus::Done)
        );
        if !allowed {
            return Err(TransitionError::new("task", self.status.as_str(), to.as_str()).into());
        }
        self.status = to;
        Ok(())
    }

    /// A run starts on this task. Refused without an attached spec.
    pub fn start(&mut self) -> Result<(), DomainError> {
        if self.spec_version_id.is_none() {
            return Err(DomainError::Invariant(
                "a task cannot start without an approved spec (spec before code)",
            ));
        }
        self.go(TaskStatus::InProgress)
    }

    /// The run finished; the reviewer takes over.
    pub fn submit_for_review(&mut self) -> Result<(), DomainError> {
        self.go(TaskStatus::Review)
    }

    /// The reviewer run starts on this task. Only from `review`, and only
    /// for the reviewer role — an executor run can never start here: the
    /// task leaves review through the human's decision, not through more
    /// work. The task itself does not move; the review's lifecycle is the
    /// `Run`'s (one active run per task, the executor's is finished by then).
    pub fn start_review(&self, role: &RoleId) -> Result<(), DomainError> {
        if self.status != TaskStatus::Review {
            return Err(DomainError::Invariant(
                "a review run starts only on a task in review",
            ));
        }
        if role.as_str() != "reviewer" {
            return Err(DomainError::Invariant(
                "only the reviewer runs on a task in review",
            ));
        }
        Ok(())
    }

    /// The run died (error or cancelled): the task goes back to the board,
    /// ready to be re-dispatched. Deliberately not folded into `go`'s table —
    /// `requeue` from in_progress must stay refused.
    pub fn fail_run(&mut self) -> Result<(), DomainError> {
        if self.status != TaskStatus::InProgress {
            return Err(TransitionError::new("task", self.status.as_str(), "ready").into());
        }
        self.status = TaskStatus::Ready;
        Ok(())
    }

    pub fn request_changes(&mut self) -> Result<(), DomainError> {
        self.go(TaskStatus::ChangesRequested)
    }

    /// Changes addressed or re-planned: back to the board.
    pub fn requeue(&mut self) -> Result<(), DomainError> {
        self.go(TaskStatus::Ready)
    }

    /// The owner approves. Requires a *granted review* approval — the
    /// invariant that nothing merges without an explicit human decision.
    pub fn approve(&mut self, approval: &Approval) -> Result<(), DomainError> {
        if approval.kind != ApprovalKind::Review {
            return Err(DomainError::Invariant(
                "a task is done only through a review approval",
            ));
        }
        if !approval.is_granted() {
            return Err(DomainError::Invariant(
                "a task is done only through a granted approval",
            ));
        }
        self.go(TaskStatus::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ApprovalId, RunId};

    fn task() -> Task {
        Task::new(
            TaskId::new("t1").unwrap(),
            ProjectId::new("p1").unwrap(),
            RoleId::new("frontend").unwrap(),
            "Page de connexion",
            "Formulaire email + mot de passe, états inclus",
            0,
        )
        .unwrap()
    }

    fn granted_review() -> Approval {
        let mut a = Approval::new(
            ApprovalId::new("a1").unwrap(),
            RunId::new("r1").unwrap(),
            ApprovalKind::Review,
            "{}".into(),
        );
        a.grant().unwrap();
        a
    }

    fn spec() -> SpecVersionId {
        SpecVersionId::new("s1").unwrap()
    }

    #[test]
    fn a_task_needs_a_title() {
        assert!(Task::new(
            TaskId::new("t1").unwrap(),
            ProjectId::new("p1").unwrap(),
            RoleId::new("backend").unwrap(),
            "  ",
            "",
            0,
        )
        .is_err());
    }

    #[test]
    fn spec_before_code_is_enforced() {
        let mut t = task();
        assert!(t.start().is_err()); // no spec attached
        t.bind_spec(spec());
        t.start().unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);
    }

    #[test]
    fn full_cycle_with_changes() {
        let mut t = task();
        t.bind_spec(spec());
        t.start().unwrap();
        t.submit_for_review().unwrap();
        t.request_changes().unwrap();
        t.requeue().unwrap();
        t.start().unwrap();
        t.submit_for_review().unwrap();
        t.approve(&granted_review()).unwrap();
        assert_eq!(t.status, TaskStatus::Done);
    }

    #[test]
    fn done_requires_a_granted_review_approval() {
        let mut t = task();
        t.bind_spec(spec());
        t.start().unwrap();
        t.submit_for_review().unwrap();

        // Wrong kind: a spec approval cannot close a task.
        let mut spec_approval = Approval::new(
            ApprovalId::new("a2").unwrap(),
            RunId::new("r1").unwrap(),
            ApprovalKind::Spec,
            "{}".into(),
        );
        spec_approval.grant().unwrap();
        assert!(t.approve(&spec_approval).is_err());

        // Right kind but not granted.
        let pending = Approval::new(
            ApprovalId::new("a3").unwrap(),
            RunId::new("r1").unwrap(),
            ApprovalKind::Review,
            "{}".into(),
        );
        assert!(t.approve(&pending).is_err());
        assert_eq!(t.status, TaskStatus::Review);
    }

    #[test]
    fn refused_transitions() {
        let mut t = task();
        t.bind_spec(spec());
        assert!(t.submit_for_review().is_err()); // Ready → Review
        assert!(t.approve(&granted_review()).is_err()); // Ready → Done
        t.start().unwrap();
        assert!(t.requeue().is_err()); // InProgress → Ready
        assert!(t.request_changes().is_err()); // InProgress → ChangesRequested
    }

    #[test]
    fn a_dead_run_sends_the_task_back_to_ready() {
        let mut t = task();
        t.bind_spec(spec());
        t.start().unwrap();
        t.fail_run().unwrap();
        assert_eq!(t.status, TaskStatus::Ready);
        // The spec binding survives: the task can start again immediately.
        t.start().unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);
    }

    #[test]
    fn fail_run_is_refused_from_everything_but_in_progress() {
        // Ready: nothing ever started.
        assert!(task().fail_run().is_err());

        // Review: the reviewer is on it — a dead executor run must not
        // bounce a task out of review.
        let mut in_review = task();
        in_review.bind_spec(spec());
        in_review.start().unwrap();
        in_review.submit_for_review().unwrap();
        assert!(in_review.fail_run().is_err());

        // ChangesRequested: requeue is the door, not fail_run.
        let mut changes = task();
        changes.bind_spec(spec());
        changes.start().unwrap();
        changes.submit_for_review().unwrap();
        changes.request_changes().unwrap();
        assert!(changes.fail_run().is_err());

        // Done: terminal.
        let mut done = task();
        done.bind_spec(spec());
        done.start().unwrap();
        done.submit_for_review().unwrap();
        done.approve(&granted_review()).unwrap();
        assert!(done.fail_run().is_err());
    }

    #[test]
    fn the_reviewer_starts_only_on_a_task_in_review() {
        let reviewer = RoleId::new("reviewer").unwrap();
        let mut t = task();
        t.bind_spec(spec());
        t.start().unwrap();
        // Not yet: the task is still being built.
        assert!(t.start_review(&reviewer).is_err());
        t.submit_for_review().unwrap();
        t.start_review(&reviewer).unwrap();
    }

    #[test]
    fn no_executor_run_from_review() {
        let mut t = task();
        t.bind_spec(spec());
        t.start().unwrap();
        t.submit_for_review().unwrap();
        // start() itself stays refused from review…
        assert!(t.start().is_err());
        // …and start_review refuses any other role.
        assert!(t.start_review(&RoleId::new("frontend").unwrap()).is_err());
        assert!(t.start_review(&RoleId::new("backend").unwrap()).is_err());
    }
}
