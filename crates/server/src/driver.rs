//! The supervision driver — the tokio half of run supervision (the decision
//! half is `latoile_app::supervision`, pure). Polls the store for active
//! runs, asks the channel what became of them, applies the plan. On a
//! finished run it also refreshes the preview (§5.2: RunFinished →
//! EnsurePreview → PreviewReady).
//!
//! Poll-based, like the SSE tail: the channel exposes `run_state` polling,
//! not a callback. The interval is short enough for a snappy UI and long
//! enough to be free — two cheap queries per tick.

use crate::state::AppState;
use latoile_agents::RunState;
use latoile_app::supervision::{self, Observed};
use latoile_app::use_cases::EnsurePreview;
use latoile_core::ids::RunId;
use latoile_core::ports::RunStore;
use std::time::Duration;
use tokio::task::JoinHandle;

pub const DEFAULT_POLL: Duration = Duration::from_secs(2);

pub fn spawn(state: AppState) -> JoinHandle<()> {
    spawn_every(state, DEFAULT_POLL)
}

pub fn spawn_every(state: AppState, interval: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(e) = tick(&state).await {
                tracing::warn!(error = %e, "supervision tick failed");
            }
            tokio::time::sleep(interval).await;
        }
    })
}

async fn tick(state: &AppState) -> Result<(), latoile_app::use_cases::UseCaseError> {
    let runs = state.store.active_runs().await?;
    for run in runs {
        let observed = observe(state, &run.id).await;
        if observed == Observed::Running {
            continue;
        }
        let applied = supervision::apply(&state.store, &run.id, &observed).await?;

        if let Some(project) = applied.project_id.clone() {
            if applied.review_approval.is_some() {
                // §5.2, in order: refresh the preview, then dispatch the
                // reviewer. Both best-effort — a dead dev server or a
                // missing adapter must not break supervision.
                let ensured = EnsurePreview::new(
                    state.store.clone(),
                    state.store.clone(),
                    state.previews.clone(),
                    state.store.clone(),
                )
                .execute(&project)
                .await;
                if let Err(e) = ensured {
                    tracing::warn!(error = %e, "preview refresh after run failed");
                }

                // The reviewer needs the finished run's summary.
                let finished = RunStore::get(&state.store, &run.id).await?;
                if let Some(finished) = finished {
                    let summary = finished.summary.clone().unwrap_or_default();
                    let reviewed = supervision::start_review(
                        &state.store,
                        &state.agents,
                        &finished.task_id,
                        &finished.id,
                        &summary,
                    )
                    .await;
                    if let Err(e) = reviewed {
                        tracing::warn!(error = %e, "reviewer dispatch failed");
                    }
                }
            }
        }
    }
    Ok(())
}

async fn observe(state: &AppState, run: &RunId) -> Observed {
    match state.agents.run_state(run).await {
        // The channel knows no such run: its process died with a restart.
        None => Observed::Lost,
        Some(RunState::Running) => Observed::Running,
        Some(RunState::Done(report)) => match report.outcome {
            latoile_agents::RunOutcome::Finished => {
                let artifacts = serde_json::to_string(&report).unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "run evidence serialization failed");
                    "{}".into()
                });
                Observed::Finished {
                    summary: report.summary,
                    base_sha: report.base_sha,
                    head_sha: report.head_sha,
                    artifacts,
                }
            }
            latoile_agents::RunOutcome::Cancelled => Observed::Cancelled,
            latoile_agents::RunOutcome::Failed => Observed::Failed {
                reason: "the agent ended the turn without finishing".into(),
            },
        },
        Some(RunState::Failed(reason)) => Observed::Failed { reason },
    }
}
