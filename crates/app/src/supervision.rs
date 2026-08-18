//! Run supervision — what happens when the agent channel reports a run has
//! ended. The decision ([`plan`]) is a pure function of the stored entities;
//! the effectful half ([`apply`]) needs only the store. The tokio polling
//! loop lives in the server crate — this module never names a runtime.
//!
//! A failed or cancelled run sends its task back to `ready`
//! (`Task::fail_run`), so a dead run never parks a task; a finished run moves
//! its task to `review` and the reviewer is dispatched ([`start_review`],
//! §5.2).

use crate::store::Store;
use crate::use_cases::UseCaseError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{ApprovalId, RoleId, RunId, TaskId};
use latoile_core::ports::{AgentChannel, ApprovalStore, EventLog, RunStore, TaskStore};
use latoile_core::{Approval, ApprovalKind, Run, RunStatus, Task, TaskStatus, TriggeredBy};

/// What the channel says about a run the store believes is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// Still going — nothing to do.
    Running,
    Finished {
        summary: String,
        base_sha: Option<String>,
        head_sha: Option<String>,
        artifacts: String,
    },
    Cancelled,
    Failed { reason: String },
    /// Active in the store but unknown to the channel: the process died
    /// with a server restart. Treated as failed.
    Lost,
}

impl Observed {
    /// Terminal observation without repository evidence. Useful for
    /// adapters that cannot inspect Git and for focused domain tests.
    pub fn finished(summary: impl Into<String>) -> Self {
        Self::Finished {
            summary: summary.into(),
            base_sha: None,
            head_sha: None,
            artifacts: "{}".into(),
        }
    }
}

/// One step, in domain-legal order, computed from the stored statuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    BeginRun,
    ResumeRun,
    FinishRun {
        summary: String,
        base_sha: Option<String>,
        head_sha: Option<String>,
        artifacts: String,
    },
    FailRun,
    CancelRun,
    SubmitForReview,
    /// Create the review approval for the human (§5.2's reviewer surface).
    RequestReviewApproval,
    /// The run died: the task goes back to the board (`Task::fail_run`).
    RequeueTask,
    Journal(EventKind, String),
}

/// The legal way to walk a run from its stored status into `finish`/`fail`/
/// `cancel`.
fn wind_down(run: &Run, terminal: Terminal) -> Vec<Step> {
    if !run.status.is_active() {
        return vec![]; // already terminal: a double tick changes nothing
    }
    let mut steps = Vec::new();
    match run.status {
        RunStatus::Starting => steps.push(Step::BeginRun),
        RunStatus::Blocked => steps.push(Step::ResumeRun),
        _ => {}
    }
    steps.push(match terminal {
        Terminal::Finish {
            summary,
            base_sha,
            head_sha,
            artifacts,
        } => Step::FinishRun {
            summary,
            base_sha,
            head_sha,
            artifacts,
        },
        Terminal::Fail => Step::FailRun,
        Terminal::Cancel => Step::CancelRun,
    });
    steps
}

enum Terminal {
    Finish {
        summary: String,
        base_sha: Option<String>,
        head_sha: Option<String>,
        artifacts: String,
    },
    Fail,
    Cancel,
}

/// What should happen, in order. Empty when nothing should change.
pub fn plan(run: &Run, task: &Task, observed: &Observed) -> Vec<Step> {
    let run_payload = format!("{{\"run_id\":\"{}\",\"outcome\":\"", run.id.as_str());
    match observed {
        Observed::Running => vec![],
        Observed::Finished {
            summary,
            base_sha,
            head_sha,
            artifacts,
        } => {
            let mut steps = wind_down(
                run,
                Terminal::Finish {
                    summary: summary.clone(),
                    base_sha: base_sha.clone(),
                    head_sha: head_sha.clone(),
                    artifacts: artifacts.clone(),
                },
            );
            if steps.is_empty() {
                return vec![]; // already recorded
            }
            steps.push(Step::Journal(
                EventKind::RunFinished,
                format!("{run_payload}finished\"}}"),
            ));
            // The task goes to review only from in_progress — and a review
            // approval is requested exactly once.
            if task.status == TaskStatus::InProgress {
                steps.push(Step::SubmitForReview);
                steps.push(Step::RequestReviewApproval);
                steps.push(Step::Journal(
                    EventKind::ApprovalRequested,
                    format!(
                        "{{\"run_id\":\"{}\",\"kind\":\"review\"}}",
                        run.id.as_str()
                    ),
                ));
            }
            steps
        }
        Observed::Cancelled => {
            let mut steps = wind_down(run, Terminal::Cancel);
            if !steps.is_empty() {
                steps.push(Step::Journal(
                    EventKind::RunFinished,
                    format!("{run_payload}cancelled\"}}"),
                ));
            }
            requeue(task, &mut steps);
            steps
        }
        Observed::Failed { reason } => fail_plan(run, task, &run_payload, reason),
        Observed::Lost => fail_plan(run, task, &run_payload, "lost to a server restart"),
    }
}

fn fail_plan(run: &Run, task: &Task, run_payload: &str, reason: &str) -> Vec<Step> {
    let mut steps = wind_down(run, Terminal::Fail);
    if !steps.is_empty() {
        steps.push(Step::Journal(
            EventKind::RunFinished,
            format!(
                "{run_payload}error\",\"reason\":{}}}",
                serde_json::Value::String(reason.to_string())
            ),
        ));
    }
    requeue(task, &mut steps);
    steps
}

/// After a dead run, an in-progress task goes back to `ready` — and the
/// board hears about it.
fn requeue(task: &Task, steps: &mut Vec<Step>) {
    if !steps.is_empty() && task.status == TaskStatus::InProgress {
        steps.push(Step::RequeueTask);
        steps.push(Step::Journal(
            EventKind::TaskReady,
            format!("{{\"task_id\":\"{}\"}}", task.id.as_str()),
        ));
    }
}

/// What `apply` did — enough for the driver to log and tests to assert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub steps: usize,
    pub review_approval: Option<ApprovalId>,
    /// The run's project — the driver needs it for the §5.2 EnsurePreview
    /// step. None when the run was unknown.
    pub project_id: Option<latoile_core::ids::ProjectId>,
}

/// Fetch, plan, execute. Idempotent: an already-terminal run plans to
/// nothing, so a repeated tick is a no-op.
pub async fn apply(store: &Store, run_id: &RunId, observed: &Observed) -> Result<Applied, UseCaseError> {
    let empty = |project_id| Applied {
        steps: 0,
        review_approval: None,
        project_id,
    };
    let Some(run) = RunStore::get(store, run_id).await? else {
        return Ok(empty(None));
    };
    let Some(task) = TaskStore::get(store, &run.task_id).await? else {
        return Ok(empty(None));
    };
    let project_id = task.project_id.clone();
    let steps = plan(&run, &task, observed);
    if steps.is_empty() {
        return Ok(empty(Some(project_id)));
    }

    let mut run = run;
    let mut task = task;
    let mut review_approval = None;
    let mut applied = 0;
    for step in steps {
        match step {
            Step::BeginRun => run.begin()?,
            Step::ResumeRun => run.resume()?,
            Step::FinishRun {
                summary,
                base_sha,
                head_sha,
                artifacts,
            } => {
                run.finish(summary)?;
                run.attach_evidence(base_sha, head_sha, artifacts)?;
            }
            Step::FailRun => run.fail()?,
            Step::CancelRun => run.cancel()?,
            Step::SubmitForReview => task.submit_for_review()?,
            Step::RequeueTask => task.fail_run()?,
            Step::RequestReviewApproval => {
                let approval = Approval::new(
                    ApprovalId::new(ulid::Ulid::new().to_string())?,
                    run.id.clone(),
                    ApprovalKind::Review,
                    serde_json::json!({
                        "summary": run.summary.clone().unwrap_or_default(),
                    })
                    .to_string(),
                );
                ApprovalStore::save(store, &approval).await?;
                review_approval = Some(approval.id);
            }
            Step::Journal(kind, payload) => {
                EventLog::append(
                    store,
                    &NewEvent {
                        project_id: task.project_id.clone(),
                        kind,
                        payload,
                    },
                )
                .await?;
            }
        }
        applied += 1;
    }
    RunStore::save(store, &run).await?;
    TaskStore::save(store, &task).await?;
    Ok(Applied {
        steps: applied,
        review_approval,
        project_id: Some(project_id),
    })
}

/// Dispatch the reviewer run on a task that just entered review (§5.2).
/// `Task::start_review` is the guard: review status, reviewer role. The
/// prompt carries the finished run's summary — the reviewer never sees the
/// conversation.
///
/// A spawn failure is journaled, not raised: the review approval already
/// exists and the human can decide without the reviewer.
pub async fn start_review<A: AgentChannel>(
    store: &Store,
    agents: &A,
    task_id: &TaskId,
    finished_run: &RunId,
    summary: &str,
) -> Result<Run, UseCaseError> {
    let task = TaskStore::get(store, task_id)
        .await?
        .ok_or(UseCaseError::NotFound("task"))?;
    let role = RoleId::new("reviewer")?;
    task.start_review(&role)?;

    let mut run = Run::new(
        RunId::new(ulid::Ulid::new().to_string())?,
        task.id.clone(),
        role,
        TriggeredBy::Manager,
    );
    let prompt = format!(
        "Review the changes produced by run {} on task {:?}. Summary of the work:\n{}",
        finished_run.as_str(),
        task.title,
        if summary.is_empty() { "(none)" } else { summary },
    );

    match agents.start_run(&run, &prompt).await {
        Ok(session) => {
            run.acp_session_id = Some(session);
            run.begin()?;
        }
        Err(e) => {
            run.fail()?;
            RunStore::save(store, &run).await?;
            EventLog::append(
                store,
                &NewEvent {
                    project_id: task.project_id.clone(),
                    kind: EventKind::RunFinished,
                    payload: format!(
                        "{{\"run_id\":\"{}\",\"outcome\":\"error\",\"reason\":{}}}",
                        run.id.as_str(),
                        serde_json::Value::String(format!("reviewer spawn failed: {e}"))
                    ),
                },
            )
            .await?;
            return Ok(run);
        }
    }

    RunStore::save(store, &run).await?;
    EventLog::append(
        store,
        &NewEvent {
            project_id: task.project_id,
            kind: EventKind::RunStarted,
            payload: format!(
                "{{\"task_id\":\"{}\",\"run_id\":\"{}\"}}",
                task.id.as_str(),
                run.id.as_str()
            ),
        },
    )
    .await?;
    Ok(run)
}


#[cfg(test)]
mod tests;
