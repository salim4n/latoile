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
use latoile_core::event::{EventKind, NewEvent};
use latoile_core::ids::RunId;
use latoile_core::ports::{
    ArchitectureSessionStore, EventLog, PreviewStore, ProjectStore, RunStore, SpecStore, TaskStore,
};
use latoile_core::{PreviewStatus, Run};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::task::JoinHandle;

pub const DEFAULT_POLL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoverySummary {
    pub runs: usize,
    pub blocked_permissions: usize,
    pub previews: usize,
    pub architecture_sessions: usize,
}

/// Reconcile process-backed rows before the HTTP listener can become ready.
/// A fresh process owns no previous ACP connection or preview registry, so
/// every active row is lost. Domain transitions make the next action clear:
/// executor tasks return to `ready`, lost Reviewer runs produce a bounded
/// changes-requested decision, pending permissions are rejected, and preview
/// rows become `error` with no pid.
pub async fn recover_startup(
    state: &AppState,
) -> Result<RecoverySummary, latoile_app::use_cases::UseCaseError> {
    let runs = state.store.active_runs().await?;
    let blocked_permissions = runs
        .iter()
        .filter(|run| run.status == latoile_core::RunStatus::Blocked)
        .count();
    for run in &runs {
        supervision::apply(&state.store, &run.id, &Observed::Lost).await?;
    }
    let mut architecture_sessions = state.store.active_architecture_sessions().await?;
    for session in &mut architecture_sessions {
        session.fail(
            "live Architect session was lost to a server restart; restart discovery from the durable answers",
        )?;
        ArchitectureSessionStore::save(&state.store, session).await?;
    }
    let previews = reconcile_previews(state, true).await?;
    let summary = RecoverySummary {
        runs: runs.len(),
        blocked_permissions,
        previews,
        architecture_sessions: architecture_sessions.len(),
    };
    if summary.runs + summary.previews + summary.architecture_sessions > 0 {
        tracing::warn!(
            runs = summary.runs,
            blocked_permissions = summary.blocked_permissions,
            previews = summary.previews,
            architecture_sessions = summary.architecture_sessions,
            "startup recovery reconciled lost process state"
        );
    }
    Ok(summary)
}

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
        // The owner decision route uses the same tiny critical section. It
        // prevents a polling tick based on a stale permission snapshot from
        // writing `blocked` back after the HTTP decision resumed the run.
        let (observed, applied) = {
            let _decision_guard = state.decision_lock.lock().await;
            let observed = observe(state, &run.id).await;
            if observed == Observed::Running {
                continue;
            }
            let applied = supervision::apply(&state.store, &run.id, &observed).await?;
            (observed, applied)
        };
        if let Observed::PermissionExpired(request) = &observed {
            state
                .agents
                .acknowledge_permission_expiry(&run.id, &request.id);
        }

        if let Some(project) = applied.project_id.clone() {
            if applied.reviewer_dispatch_requested {
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

                // The reviewer gets task, approved spec references/excerpts,
                // visual-contract paths and sanitized Git evidence.
                let finished = RunStore::get(&state.store, &run.id).await?;
                if let Some(finished) = finished {
                    let context = review_context(state, &finished).await?;
                    let reviewed = supervision::start_review(
                        &state.store,
                        &state.agents,
                        &finished.task_id,
                        &finished.id,
                        &context,
                    )
                    .await;
                    if let Err(e) = reviewed {
                        tracing::warn!(error = %e, "reviewer dispatch failed");
                    }
                }
            }
        }
    }
    reconcile_previews(state, false).await?;
    Ok(())
}

/// Runtime health reconciliation checks only `ready` previews. A persisted
/// `stale` row may legitimately be between process recycle and readiness;
/// startup passes `all_active = true` because a new registry cannot own any
/// pre-restart process.
async fn reconcile_previews(
    state: &AppState,
    all_active: bool,
) -> Result<usize, latoile_app::use_cases::UseCaseError> {
    let previews = state.store.active_previews().await?;
    let mut reconciled = 0;
    for mut preview in previews {
        if !all_active && preview.status != PreviewStatus::Ready {
            continue;
        }
        if state.previews.is_alive(&preview.id).await {
            continue;
        }
        preview.fail()?;
        PreviewStore::save(&state.store, &preview).await?;
        EventLog::append(
            &state.store,
            &NewEvent {
                project_id: preview.project_id.clone(),
                kind: EventKind::PreviewError,
                payload: serde_json::json!({
                    "preview_id": preview.id.as_str(),
                    "reason": if all_active { "server_restart" } else { "process_exited" },
                    "next_action": "restart_preview",
                })
                .to_string(),
            },
        )
        .await?;
        reconciled += 1;
    }
    Ok(reconciled)
}

async fn review_context(
    state: &AppState,
    finished: &Run,
) -> Result<String, latoile_app::use_cases::UseCaseError> {
    let task = TaskStore::get(&state.store, &finished.task_id)
        .await?
        .ok_or(latoile_app::use_cases::UseCaseError::NotFound("task"))?;
    let spec = SpecStore::approved_for_project(&state.store, &task.project_id).await?;
    let project = ProjectStore::get(&state.store, &task.project_id)
        .await?
        .ok_or(latoile_app::use_cases::UseCaseError::NotFound("project"))?;

    let (spec_reference, excerpts, visual_references) = match spec {
        Some(spec) => {
            let (excerpts, visuals) = design_evidence(&project.local_path, &spec.design_dir);
            (
                format!(
                    "spec {} v{} (approved), directory `{}`",
                    spec.id.as_str(),
                    spec.version,
                    spec.design_dir
                ),
                excerpts,
                visuals,
            )
        }
        None => (
            "approved spec unavailable".into(),
            "(no approved spec excerpt available)".into(),
            "(no visual-contract reference available)".into(),
        ),
    };
    let base = finished.base_sha.as_deref().unwrap_or("unknown");
    let head = finished.head_sha.as_deref().unwrap_or("unknown");
    let artifacts = finished.artifacts.as_deref().unwrap_or("{}");

    Ok(format!(
        "TASK\n- id: {}\n- role: {}\n- title: {}\n- description: {}\n\nAPPROVED SPEC\n{}\n\nSPEC EXCERPTS\n{}\n\nVISUAL CONTRACT REFERENCES\n{}\n\nEXECUTION EVIDENCE\n- summary: {}\n- base SHA: {}\n- head SHA: {}\n- sanitized artifacts: {}\n\nInspect the repository diff between the two SHAs (plus working-tree changes). For frontend work, compare the live render with the visual references before issuing the verdict.",
        task.id.as_str(),
        task.role_id.as_str(),
        task.title,
        task.description,
        spec_reference,
        excerpts,
        visual_references,
        finished.summary.as_deref().unwrap_or("(none)"),
        base,
        head,
        truncate(artifacts, 32 * 1024),
    ))
}

/// Read only bounded, text-shaped spec evidence under the project's
/// checkout. A stale or escaping design path degrades to metadata only.
fn design_evidence(local_path: &str, design_dir: &str) -> (String, String) {
    let Ok(root) = std::fs::canonicalize(local_path) else {
        return unavailable_design_evidence();
    };
    let Ok(design) = std::fs::canonicalize(root.join(design_dir)) else {
        return unavailable_design_evidence();
    };
    if !design.starts_with(&root) {
        return unavailable_design_evidence();
    }

    let mut files = collect_design_files(&design);
    files.sort();
    let mut excerpts = String::new();
    let mut visuals = Vec::new();
    for path in files.into_iter().take(40) {
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
            "md" | "txt" | "json" | "yaml" | "yml" if excerpts.len() < 16 * 1024 => {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    excerpts.push_str(&format!(
                        "\n### `{relative}`\n{}\n",
                        truncate(&content, 4 * 1024)
                    ));
                }
            }
            "png" | "jpg" | "jpeg" | "webp" | "svg" => visuals.push(relative),
            _ => {}
        }
    }

    if excerpts.is_empty() {
        excerpts.push_str("(no readable text spec file found)");
    }
    let visuals = if visuals.is_empty() {
        "(no image reference found; inspect the design directory directly)".into()
    } else {
        visuals
            .into_iter()
            .take(20)
            .map(|path| format!("- `{path}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    (truncate(&excerpts, 20 * 1024), visuals)
}

fn collect_design_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut visited_dirs = 0usize;
    while let Some(dir) = pending.pop() {
        visited_dirs += 1;
        if visited_dirs > 40 || files.len() >= 100 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() && visited_dirs + pending.len() < 40 {
                pending.push(path);
            } else if file_type.is_file() {
                files.push(path);
            }
        }
    }
    files
}

fn unavailable_design_evidence() -> (String, String) {
    (
        "(spec files unavailable; inspect the approved design directory in the workspace)".into(),
        "(visual references unavailable)".into(),
    )
}

fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut boundary = limit;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}…", &value[..boundary])
}

async fn observe(state: &AppState, run: &RunId) -> Observed {
    match state.agents.run_state(run).await {
        // The channel knows no such run: its process died with a restart.
        None => Observed::Lost,
        Some(RunState::Running) => Observed::Running,
        Some(RunState::Blocked(request)) => Observed::PermissionRequested(request),
        Some(RunState::PermissionExpired(request)) => Observed::PermissionExpired(request),
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

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn design_evidence_includes_bounded_text_and_visual_references() {
        let checkout = tempfile::tempdir().unwrap();
        let design = checkout.path().join("design");
        std::fs::create_dir_all(&design).unwrap();
        std::fs::write(
            design.join("spec.md"),
            "# Login\n\nLe bouton primaire respecte un espacement de 16 px.",
        )
        .unwrap();
        std::fs::write(design.join("login.png"), b"not-a-real-image").unwrap();

        let (excerpts, visuals) = design_evidence(checkout.path().to_str().unwrap(), "design");
        assert!(excerpts.contains("Le bouton primaire"));
        assert!(excerpts.contains("design/spec.md"));
        assert!(visuals.contains("design/login.png"));
    }

    #[test]
    fn an_escaping_design_path_is_never_read() {
        let checkout = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.md"), "must not leak").unwrap();

        let (excerpts, _) = design_evidence(
            checkout.path().to_str().unwrap(),
            outside.path().to_str().unwrap(),
        );
        assert!(!excerpts.contains("must not leak"));
        assert!(excerpts.contains("unavailable"));
    }
}
