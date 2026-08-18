//! Run supervision — what happens when the agent channel reports a run has
//! ended. The decision ([`plan`]) is a pure function of the stored entities;
//! the effectful half ([`apply`]) needs only the store. The tokio polling
//! loop lives in the server crate — this module never names a runtime.
//!
//! A failed or cancelled run sends its task back to `ready`
//! (`Task::fail_run`), so a dead run never parks a task; a finished run moves
//! its task to `review` and the reviewer is dispatched ([`start_review`],
//! §5.2).

use crate::review_result::{review_failure_payload, trusted_review_payload, ReviewTrustContext};
use crate::store::Store;
use crate::use_cases::UseCaseError;
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::{ApprovalId, RoleId, RunId, TaskId};
use latoile_core::ports::PermissionRequest;
use latoile_core::ports::{
    AgentChannel, ApprovalStore, EventLog, RunStore, SpecStore, TaskStore, VisualComparisonStore,
};
use latoile_core::{Approval, ApprovalKind, Run, RunStatus, Task, TaskStatus, TriggeredBy};

/// What the channel says about a run the store believes is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// Still going — nothing to do.
    Running,
    PermissionRequested(PermissionRequest),
    PermissionExpired(PermissionRequest),
    Finished {
        summary: String,
        base_sha: Option<String>,
        head_sha: Option<String>,
        artifacts: String,
    },
    Cancelled,
    Failed {
        reason: String,
    },
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
    BlockRun,
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
    /// Tell the server driver to start the Reviewer after preview refresh.
    DispatchReviewer,
    /// Create the human approval from a terminal Reviewer result.
    RequestReviewApproval {
        output: String,
        failure_reason: Option<String>,
    },
    RequestPermissionApproval {
        request: PermissionRequest,
    },
    RejectPermissionApproval {
        request: PermissionRequest,
        reason: String,
    },
    RejectPendingPermissions {
        reason: String,
    },
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
        Observed::Running => {
            if run.status == RunStatus::Blocked {
                vec![Step::ResumeRun]
            } else {
                vec![]
            }
        }
        Observed::PermissionRequested(request) => {
            if !run.status.is_active() {
                return vec![];
            }
            let mut steps = Vec::new();
            if run.status == RunStatus::Starting {
                steps.push(Step::BeginRun);
            }
            if run.status != RunStatus::Blocked {
                steps.push(Step::BlockRun);
                steps.push(Step::RequestPermissionApproval {
                    request: request.clone(),
                });
                steps.push(Step::Journal(
                    EventKind::RunBlocked,
                    serde_json::json!({
                        "run_id": run.id.as_str(),
                        "permission_request_id": request.id,
                    })
                    .to_string(),
                ));
                steps.push(Step::Journal(
                    EventKind::ApprovalRequested,
                    serde_json::json!({
                        "run_id": run.id.as_str(),
                        "kind": "permission",
                        "permission_request_id": request.id,
                    })
                    .to_string(),
                ));
            } else {
                steps.push(Step::RequestPermissionApproval {
                    request: request.clone(),
                });
            }
            steps
        }
        Observed::PermissionExpired(request) => {
            if !run.status.is_active() {
                return vec![];
            }
            let mut steps = Vec::new();
            if run.status == RunStatus::Starting {
                steps.push(Step::BeginRun);
            }
            if run.status != RunStatus::Blocked {
                steps.push(Step::BlockRun);
                steps.push(Step::RequestPermissionApproval {
                    request: request.clone(),
                });
                steps.push(Step::Journal(
                    EventKind::RunBlocked,
                    serde_json::json!({
                        "run_id": run.id.as_str(),
                        "permission_request_id": request.id,
                        "expired": true,
                    })
                    .to_string(),
                ));
                steps.push(Step::Journal(
                    EventKind::ApprovalRequested,
                    serde_json::json!({
                        "run_id": run.id.as_str(),
                        "kind": "permission",
                        "permission_request_id": request.id,
                        "expired": true,
                    })
                    .to_string(),
                ));
            } else {
                steps.push(Step::RequestPermissionApproval {
                    request: request.clone(),
                });
            }
            steps.push(Step::RejectPermissionApproval {
                request: request.clone(),
                reason: "permission request timed out".into(),
            });
            steps.push(Step::ResumeRun);
            steps
        }
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
            if run.role_id.as_str() == "reviewer" && task.status == TaskStatus::Review {
                steps.push(Step::RequestReviewApproval {
                    output: summary.clone(),
                    failure_reason: None,
                });
                steps.push(Step::Journal(
                    EventKind::ApprovalRequested,
                    format!("{{\"run_id\":\"{}\",\"kind\":\"review\"}}", run.id.as_str()),
                ));
            } else if task.status == TaskStatus::InProgress {
                // Executor finished: enter review and dispatch the Reviewer.
                // No human approval exists until that run terminates.
                steps.push(Step::SubmitForReview);
                steps.push(Step::DispatchReviewer);
            }
            steps
        }
        Observed::Cancelled => {
            let mut steps = wind_down(run, Terminal::Cancel);
            if !steps.is_empty() {
                steps.push(Step::RejectPendingPermissions {
                    reason: "run cancelled while awaiting permission".into(),
                });
                steps.push(Step::Journal(
                    EventKind::RunFinished,
                    format!("{run_payload}cancelled\"}}"),
                ));
            }
            finish_failed_review(run, task, &mut steps, "Reviewer run cancelled");
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
        steps.push(Step::RejectPendingPermissions {
            reason: format!("permission session unavailable: {reason}"),
        });
        steps.push(Step::Journal(
            EventKind::RunFinished,
            format!(
                "{run_payload}error\",\"reason\":{}}}",
                serde_json::Value::String(reason.to_string())
            ),
        ));
    }
    finish_failed_review(run, task, &mut steps, reason);
    requeue(task, &mut steps);
    steps
}

fn finish_failed_review(run: &Run, task: &Task, steps: &mut Vec<Step>, reason: &str) {
    if !steps.is_empty() && run.role_id.as_str() == "reviewer" && task.status == TaskStatus::Review
    {
        steps.push(Step::RequestReviewApproval {
            output: String::new(),
            failure_reason: Some(reason.to_string()),
        });
        steps.push(Step::Journal(
            EventKind::ApprovalRequested,
            format!(
                "{{\"run_id\":\"{}\",\"kind\":\"review\",\"fallback\":true}}",
                run.id.as_str()
            ),
        ));
    }
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
    pub reviewer_dispatch_requested: bool,
    /// The run's project — the driver needs it for the §5.2 EnsurePreview
    /// step. None when the run was unknown.
    pub project_id: Option<latoile_core::ids::ProjectId>,
}

/// Fetch, plan, execute. Idempotent: an already-terminal run plans to
/// nothing, so a repeated tick is a no-op.
pub async fn apply(
    store: &Store,
    run_id: &RunId,
    observed: &Observed,
) -> Result<Applied, UseCaseError> {
    let empty = |project_id| Applied {
        steps: 0,
        review_approval: None,
        reviewer_dispatch_requested: false,
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
    let mut reviewer_dispatch_requested = false;
    let mut applied = 0;
    for step in steps {
        match step {
            Step::BeginRun => run.begin()?,
            Step::BlockRun => run.block()?,
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
            Step::DispatchReviewer => reviewer_dispatch_requested = true,
            Step::RequeueTask => task.fail_run()?,
            Step::RequestReviewApproval {
                output,
                failure_reason,
            } => {
                // Persist the Reviewer's terminal state before exposing its
                // approval. The deterministic id makes a retry an upsert,
                // never a second decision card.
                RunStore::save(store, &run).await?;
                TaskStore::save(store, &task).await?;
                let payload = match failure_reason {
                    Some(reason) => review_failure_payload(&reason),
                    None => gate_review_output(store, &run, &task, &output).await?,
                };
                let approval = Approval::new(
                    review_approval_id(&run.id)?,
                    run.id.clone(),
                    ApprovalKind::Review,
                    payload,
                );
                ApprovalStore::save(store, &approval).await?;
                review_approval = Some(approval.id);
            }
            Step::RequestPermissionApproval { request } => {
                RunStore::save(store, &run).await?;
                let approval = Approval::new(
                    permission_approval_id(&request.id)?,
                    run.id.clone(),
                    ApprovalKind::Permission,
                    serde_json::json!({
                        "schema_version": 1,
                        "request_id": request.id,
                        "summary": request.summary,
                    })
                    .to_string(),
                );
                ApprovalStore::save(store, &approval).await?;
            }
            Step::RejectPermissionApproval { request, reason } => {
                let id = permission_approval_id(&request.id)?;
                if let Some(mut approval) = ApprovalStore::get(store, &id).await? {
                    if approval.status == latoile_core::ApprovalStatus::Pending {
                        approval.reject_with_comment(Some(reason.clone()))?;
                        ApprovalStore::save(store, &approval).await?;
                        EventLog::append(
                            store,
                            &NewEvent {
                                project_id: task.project_id.clone(),
                                kind: EventKind::ApprovalRejected,
                                payload: serde_json::json!({
                                    "approval_id": approval.id.as_str(),
                                    "reason": reason,
                                })
                                .to_string(),
                            },
                        )
                        .await?;
                    }
                }
            }
            Step::RejectPendingPermissions { reason } => {
                for mut approval in store.pending_permissions_for_run(&run.id).await? {
                    approval.reject_with_comment(Some(reason.clone()))?;
                    ApprovalStore::save(store, &approval).await?;
                    EventLog::append(
                        store,
                        &NewEvent {
                            project_id: task.project_id.clone(),
                            kind: EventKind::ApprovalRejected,
                            payload: serde_json::json!({
                                "approval_id": approval.id.as_str(),
                                "reason": reason,
                            })
                            .to_string(),
                        },
                    )
                    .await?;
                }
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
        reviewer_dispatch_requested,
        project_id: Some(project_id),
    })
}

async fn gate_review_output(
    store: &Store,
    reviewer: &Run,
    task: &Task,
    output: &str,
) -> Result<String, UseCaseError> {
    let Some(reviewed_run_id) = reviewer.reviewed_run_id.as_ref() else {
        return Ok(review_failure_payload(
            "Reviewer V2 is not bound to an executor run; relaunch the review",
        ));
    };
    let Some(reviewed) = RunStore::get(store, reviewed_run_id).await? else {
        return Ok(review_failure_payload(
            "the executor run bound to Reviewer V2 no longer exists",
        ));
    };
    if reviewed.task_id != task.id
        || reviewed.status != RunStatus::Finished
        || reviewed.role_id.as_str() == "reviewer"
    {
        return Ok(review_failure_payload(
            "Reviewer V2 subject is not a finished executor run on this task",
        ));
    }

    let approved = SpecStore::approved_for_project(store, &task.project_id).await?;
    let current_spec_id = approved
        .as_ref()
        .and_then(|spec| (task.spec_version_id.as_ref() == Some(&spec.id)).then_some(&spec.id));
    let evidence = VisualComparisonStore::list_for_run(store, reviewed_run_id).await?;
    Ok(trusted_review_payload(
        output,
        &ReviewTrustContext {
            project_id: &task.project_id,
            spec_version_id: current_spec_id,
            reviewed_run_id,
            visual_required: reviewed.role_id.as_str() == "frontend",
            evidence: &evidence,
        },
    ))
}

fn review_approval_id(run_id: &RunId) -> Result<ApprovalId, UseCaseError> {
    Ok(ApprovalId::new(format!("review-{}", run_id.as_str()))?)
}

fn permission_approval_id(request_id: &str) -> Result<ApprovalId, UseCaseError> {
    Ok(ApprovalId::new(format!("permission-{request_id}"))?)
}

/// Dispatch the reviewer run on a task that just entered review (§5.2).
/// `Task::start_review` is the guard: review status, reviewer role. The
/// prompt carries a bounded, repository-grounded context assembled by the
/// server — the reviewer never needs the private Manager conversation.
///
/// A spawn failure is terminal and creates an honest fallback approval tied
/// to the failed Reviewer run.
pub async fn start_review<A: AgentChannel>(
    store: &Store,
    agents: &A,
    task_id: &TaskId,
    finished_run: &RunId,
    context: &str,
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
    let subject = RunStore::get(store, finished_run)
        .await?
        .ok_or(UseCaseError::NotFound("finished run"))?;
    if subject.task_id != task.id
        || subject.status != RunStatus::Finished
        || subject.role_id.as_str() == "reviewer"
    {
        return Err(latoile_core::DomainError::Invariant(
            "a review subject must be a finished executor run on the same task",
        )
        .into());
    }
    run.bind_review_subject(subject.id)?;
    let prompt = format!(
        "Review the changes produced by run {} on task {:?}.\n\n{}\n\nVERDICT RUBRIC\n- `changes_requested`: only for a concrete blocking correctness, security, approved-spec or task-acceptance defect; include at least one blocking finding with an actionable location and fix.\n- `approve_with_reservations`: the implementation is deliverable and every remaining finding is non-blocking.\n- `approve`: the implementation satisfies the task and approved specification with no finding.\nPassed visual evidence is necessary for frontend delivery but does not override a concrete blocking code defect. Optional enhancements, framework preferences and unstated scope are never blocking.\n\nReturn exactly one fenced `latoile-review` JSON block conforming to schema_version 2. Required fields: schema_version, verdict, summary, findings, suggested_follow_ups, visual_evidence. For a frontend run set visual_evidence.applicability to `required` and references to an empty array: the server binds the complete evidence set from this immutable reviewed run, so never copy ids or hashes. For a non-visual run set applicability to `not_applicable` with an empty references array. Never emit status, metrics, target/render frames or a trust gate: the server owns those facts.",
        finished_run.as_str(),
        task.title,
        if context.is_empty() { "(no execution context available)" } else { context },
    );

    match agents.start_run(&task.project_id, &run, &prompt).await {
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
            let payload = review_failure_payload(&format!("Reviewer spawn failed: {e}"));
            let approval = Approval::new(
                review_approval_id(&run.id)?,
                run.id.clone(),
                ApprovalKind::Review,
                payload,
            );
            ApprovalStore::save(store, &approval).await?;
            EventLog::append(
                store,
                &NewEvent {
                    project_id: task.project_id.clone(),
                    kind: EventKind::ApprovalRequested,
                    payload: format!(
                        "{{\"run_id\":\"{}\",\"kind\":\"review\",\"fallback\":true}}",
                        run.id.as_str()
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
