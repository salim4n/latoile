//! Replay approved visual scenarios against one finished frontend run. The
//! renderer computes evidence; this use case binds it to exact run/spec/
//! baseline provenance and persists invalid attempts honestly.

use super::UseCaseError;
use crate::store::Store;
use latoile_core::ids::{ProjectId, RunId, VisualComparisonId};
use latoile_core::ports::{
    AgentChannel, RunStore, SpecStore, TaskStore, VisualBaselineStore, VisualComparisonRenderer,
    VisualComparisonStore,
};
use latoile_core::{
    DomainError, RunStatus, VisualComparison, VisualComparisonCaptureOutcome,
    VisualComparisonCaptureRequest,
};

pub struct CaptureVisualComparisons<A, R> {
    store: Store,
    agents: A,
    renderer: R,
}

impl<A: AgentChannel, R: VisualComparisonRenderer> CaptureVisualComparisons<A, R> {
    pub fn new(store: Store, agents: A, renderer: R) -> Self {
        Self {
            store,
            agents,
            renderer,
        }
    }

    pub async fn execute(
        &self,
        project_id: &ProjectId,
        run_id: &RunId,
        live_base_url: &str,
    ) -> Result<Vec<VisualComparison>, UseCaseError> {
        let run = RunStore::get(&self.store, run_id)
            .await?
            .ok_or(UseCaseError::NotFound("run"))?;
        if run.status != RunStatus::Finished {
            return Err(DomainError::Invariant(
                "visual comparison requires a finished executor run",
            )
            .into());
        }
        let task = TaskStore::get(&self.store, &run.task_id)
            .await?
            .ok_or(UseCaseError::NotFound("task"))?;
        if &task.project_id != project_id || run.role_id.as_str() != "frontend" {
            return Err(DomainError::Invariant(
                "visual comparison applies only to a frontend run in this project",
            )
            .into());
        }
        let spec = SpecStore::approved_for_project(&self.store, project_id)
            .await?
            .ok_or(DomainError::Invariant(
                "visual comparison requires an approved architecture",
            ))?;
        if task.spec_version_id.as_ref() != Some(&spec.id) {
            return Err(DomainError::Invariant(
                "the frontend run is not bound to the currently approved spec",
            )
            .into());
        }
        let provenance = spec.provenance.as_ref().ok_or(DomainError::Invariant(
            "visual comparison requires immutable spec provenance",
        ))?;
        let validation = self
            .agents
            .verify_architecture_package(project_id, &spec)
            .await?;
        if !validation.valid || validation.scenarios.is_empty() {
            return Err(DomainError::Invariant(
                "visual comparison requires a currently valid P0 package",
            )
            .into());
        }

        let mut results = Vec::with_capacity(validation.scenarios.len());
        for scenario in validation.scenarios {
            let baseline = VisualBaselineStore::get(&self.store, &spec.id, &scenario.comparison_id)
                .await?
                .ok_or(DomainError::Invariant(
                    "required immutable visual baseline is missing",
                ))?;
            if !baseline.satisfies(
                &spec.id,
                &provenance.manifest_digest,
                &provenance.package_commit_sha,
                &scenario.comparison_id,
            ) {
                return Err(DomainError::Invariant(
                    "required visual baseline does not match the approved spec",
                )
                .into());
            }
            let id = VisualComparisonId::new(format!(
                "visual:{}:{}",
                run.id.as_str(),
                scenario.comparison_id
            ))?;
            if let Some(current) = VisualComparisonStore::get(&self.store, &id).await? {
                let provenance_matches = current.spec_version_id == spec.id
                    && current.project_id == *project_id
                    && current.run_id == run.id
                    && current.comparison_id == scenario.comparison_id
                    && current.manifest_digest == provenance.manifest_digest
                    && current.package_commit_sha == provenance.package_commit_sha
                    && current.baseline_png_digest == baseline.png_digest.as_deref().unwrap_or("");
                if !provenance_matches {
                    return Err(DomainError::Invariant(
                        "a visual comparison changed immutable provenance",
                    )
                    .into());
                }
                if current.status.has_trusted_evidence() {
                    self.renderer.verify_comparison(&current).await?;
                    results.push(current);
                    continue;
                }
            }
            let request = VisualComparisonCaptureRequest {
                id,
                spec_version_id: spec.id.clone(),
                project_id: project_id.clone(),
                run_id: run.id.clone(),
                manifest_digest: provenance.manifest_digest.clone(),
                package_commit_sha: provenance.package_commit_sha.clone(),
                baseline,
                scenario,
                live_base_url: live_base_url.to_string(),
            };
            let comparison = match self.renderer.compare(&request).await? {
                VisualComparisonCaptureOutcome::Ready(captured) => {
                    VisualComparison::ready(&request, &captured)?
                }
                VisualComparisonCaptureOutcome::Invalid {
                    code,
                    message,
                    recovery_action,
                } => VisualComparison::invalid(&request, code, message, recovery_action)?,
            };
            VisualComparisonStore::save(&self.store, &comparison).await?;
            results.push(comparison);
        }
        results.sort_by(|left, right| left.comparison_id.cmp(&right.comparison_id));
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ids::SpecVersionId;
    use latoile_core::ports::{ManagerReply, PortResult};
    use latoile_core::{
        CapturedVisualComparison, Run, SpecVersion, VisualComparisonCaptureOutcome,
        VisualComparisonCaptureRequest, VisualComparisonStatus,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct FakeAgents;

    impl AgentChannel for FakeAgents {
        async fn tell_manager(
            &self,
            _project: &ProjectId,
            _message: &str,
        ) -> PortResult<ManagerReply> {
            Err(latoile_core::ports::PortError(
                "manager is outside this test".into(),
            ))
        }

        async fn verify_architecture_package(
            &self,
            _project: &ProjectId,
            spec: &SpecVersion,
        ) -> PortResult<latoile_core::ArchitecturePackageValidation> {
            Ok(test_fixtures::test_verification(spec))
        }

        async fn start_run(
            &self,
            _project: &ProjectId,
            _run: &Run,
            _prompt: &str,
        ) -> PortResult<String> {
            Err(latoile_core::ports::PortError(
                "run start is outside this test".into(),
            ))
        }

        async fn cancel_run(&self, _run: &RunId) -> PortResult<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FakeRenderer {
        calls: Arc<AtomicUsize>,
        invalid: bool,
    }

    impl VisualComparisonRenderer for FakeRenderer {
        async fn compare(
            &self,
            _request: &VisualComparisonCaptureRequest,
        ) -> PortResult<VisualComparisonCaptureOutcome> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.invalid {
                return Ok(VisualComparisonCaptureOutcome::Invalid {
                    code: "readiness_timeout".into(),
                    message: "live route not ready".into(),
                    recovery_action: "fix route and rerun".into(),
                });
            }
            Ok(VisualComparisonCaptureOutcome::Ready(
                CapturedVisualComparison {
                    changed_pixels: 0,
                    total_pixels: 390 * 844,
                    max_geometry_delta_milli: 0,
                    accessibility_changes: 0,
                    render_png_digest: "1".repeat(64),
                    pixel_diff_digest: "2".repeat(64),
                    heatmap_png_digest: "3".repeat(64),
                    geometry_diff_digest: "4".repeat(64),
                    accessibility_diff_digest: "5".repeat(64),
                    environment_digest: "6".repeat(64),
                    browser_version: "Chrome/151".into(),
                    font_fingerprint: "b".repeat(64),
                },
            ))
        }

        async fn read_render_png(&self, _comparison: &VisualComparison) -> PortResult<Vec<u8>> {
            Ok(Vec::new())
        }

        async fn read_heatmap_png(&self, _comparison: &VisualComparison) -> PortResult<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn a_finished_frontend_run_gets_immutable_server_classified_evidence() {
        let store = test_fixtures::store_with_finished_frontend_run().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let use_case = CaptureVisualComparisons::new(
            store,
            FakeAgents,
            FakeRenderer {
                calls: calls.clone(),
                invalid: false,
            },
        );
        let run = RunId::new(test_fixtures::FINISHED_RUN).unwrap();
        let first = use_case
            .execute(&test_fixtures::PROJECT, &run, "http://127.0.0.1:4100")
            .await
            .unwrap();
        let second = use_case
            .execute(&test_fixtures::PROJECT, &run, "http://127.0.0.1:4100")
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first[0].status, VisualComparisonStatus::Passed);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first[0].spec_version_id, SpecVersionId::new("s1").unwrap());
    }

    #[tokio::test]
    async fn an_invalid_live_capture_is_stored_without_similarity_metrics() {
        let store = test_fixtures::store_with_finished_frontend_run().await;
        let rows = CaptureVisualComparisons::new(
            store,
            FakeAgents,
            FakeRenderer {
                calls: Arc::new(AtomicUsize::new(0)),
                invalid: true,
            },
        )
        .execute(
            &test_fixtures::PROJECT,
            &RunId::new(test_fixtures::FINISHED_RUN).unwrap(),
            "http://127.0.0.1:4100",
        )
        .await
        .unwrap();
        assert_eq!(rows[0].status, VisualComparisonStatus::Invalid);
        assert_eq!(rows[0].total_pixels, 0);
        assert!(rows[0].recovery_action.is_some());
    }
}
