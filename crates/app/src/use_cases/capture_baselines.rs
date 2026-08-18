//! Capture every required P0 mockup before owner approval. Package bytes are
//! revalidated and read from the pinned Git commit; successful browser
//! evidence is idempotent and immutable, while failures stay actionable and
//! retryable.

use super::UseCaseError;
use crate::store::Store;
use latoile_core::ids::SpecVersionId;
use latoile_core::ports::{AgentChannel, VisualBaselineRenderer, VisualBaselineStore};
use latoile_core::{
    DomainError, SpecStatus, VisualBaseline, VisualBaselineCaptureOutcome,
    VisualBaselineCaptureRequest,
};

pub struct CaptureBaselines<A, R> {
    store: Store,
    agents: A,
    renderer: R,
}

impl<A: AgentChannel, R: VisualBaselineRenderer> CaptureBaselines<A, R> {
    pub fn new(store: Store, agents: A, renderer: R) -> Self {
        Self {
            store,
            agents,
            renderer,
        }
    }

    pub async fn execute(&self, id: &SpecVersionId) -> Result<Vec<VisualBaseline>, UseCaseError> {
        let spec = self
            .store
            .spec_by_id(id)
            .await?
            .ok_or(UseCaseError::NotFound("spec version"))?;
        if spec.status == SpecStatus::Superseded {
            return Err(DomainError::Invariant(
                "a superseded architecture version cannot create baselines",
            )
            .into());
        }
        let provenance = spec.provenance.as_ref().ok_or({
            DomainError::Invariant("visual baselines require immutable spec provenance")
        })?;
        let validation = self
            .agents
            .verify_architecture_package(&spec.project_id, &spec)
            .await?;
        if !validation.valid || validation.scenarios.is_empty() {
            return Err(DomainError::Invariant(
                "visual baselines require a currently valid package with P0 scenarios",
            )
            .into());
        }

        let mut results = Vec::with_capacity(validation.scenarios.len());
        for scenario in validation.scenarios {
            if let Some(current) =
                VisualBaselineStore::get(&self.store, &spec.id, &scenario.comparison_id).await?
            {
                if current.satisfies(
                    &spec.id,
                    &provenance.manifest_digest,
                    &provenance.package_commit_sha,
                    &scenario.comparison_id,
                ) {
                    self.renderer.verify(&current).await?;
                    results.push(current);
                    continue;
                }
                if current.status == latoile_core::VisualBaselineStatus::Ready {
                    return Err(DomainError::Invariant(
                        "an immutable baseline does not match the current spec provenance",
                    )
                    .into());
                }
            }

            let html = self
                .agents
                .read_architecture_artifact(&spec.project_id, &spec, &scenario.mockup)
                .await?;
            let request = VisualBaselineCaptureRequest {
                spec_version_id: spec.id.clone(),
                project_id: spec.project_id.clone(),
                manifest_digest: provenance.manifest_digest.clone(),
                package_commit_sha: provenance.package_commit_sha.clone(),
                scenario,
                html,
            };
            let baseline = match self.renderer.capture(&request).await? {
                VisualBaselineCaptureOutcome::Ready(captured) => {
                    VisualBaseline::ready(&request, &captured)?
                }
                VisualBaselineCaptureOutcome::Failed {
                    code,
                    message,
                    recovery_action,
                } => VisualBaseline::failed(&request, code, message, recovery_action)?,
            };
            VisualBaselineStore::save(&self.store, &baseline).await?;
            results.push(baseline);
        }
        results.sort_by(|left, right| left.comparison_id.cmp(&right.comparison_id));
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;
    use latoile_core::ids::{ProjectId, RunId};
    use latoile_core::ports::{ManagerReply, PortResult};
    use latoile_core::{CapturedVisualBaseline, Run, SpecVersion};
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
            unimplemented!()
        }

        async fn verify_architecture_package(
            &self,
            _project: &ProjectId,
            spec: &SpecVersion,
        ) -> PortResult<latoile_core::ArchitecturePackageValidation> {
            Ok(test_fixtures::test_verification(spec))
        }

        async fn read_architecture_artifact(
            &self,
            _project: &ProjectId,
            _spec: &SpecVersion,
            _relative_path: &str,
        ) -> PortResult<String> {
            Ok("<!doctype html><html><body><main>Stable</main></body></html>".into())
        }

        async fn start_run(
            &self,
            _project: &ProjectId,
            _run: &Run,
            _prompt: &str,
        ) -> PortResult<String> {
            unimplemented!()
        }

        async fn cancel_run(&self, _run: &RunId) -> PortResult<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FakeRenderer {
        calls: Arc<AtomicUsize>,
        failed: bool,
    }

    impl VisualBaselineRenderer for FakeRenderer {
        async fn capture(
            &self,
            _request: &VisualBaselineCaptureRequest,
        ) -> PortResult<VisualBaselineCaptureOutcome> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.failed {
                Ok(VisualBaselineCaptureOutcome::Failed {
                    code: "readiness_timeout".into(),
                    message: "not ready".into(),
                    recovery_action: "fix selector".into(),
                })
            } else {
                Ok(VisualBaselineCaptureOutcome::Ready(
                    CapturedVisualBaseline {
                        png_digest: "d".repeat(64),
                        geometry_digest: "e".repeat(64),
                        accessibility_digest: "f".repeat(64),
                        environment_digest: "a".repeat(64),
                        browser_version: "Chrome/151".into(),
                        font_fingerprint: "b".repeat(64),
                    },
                ))
            }
        }

        async fn read_png(&self, _baseline: &VisualBaseline) -> PortResult<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn capture_is_complete_and_idempotent_for_an_immutable_spec() {
        let store = test_fixtures::store_with_approved_spec_without_baseline().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let renderer = FakeRenderer {
            calls: calls.clone(),
            failed: false,
        };
        let use_case = CaptureBaselines::new(store, FakeAgents, renderer);
        let id = SpecVersionId::new(test_fixtures::SPEC).unwrap();

        let first = use_case.execute(&id).await.unwrap();
        let second = use_case.execute(&id).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first[0].status, latoile_core::VisualBaselineStatus::Ready);
    }

    #[tokio::test]
    async fn failed_capture_is_persisted_with_a_recovery_action() {
        let store = test_fixtures::store_with_approved_spec_without_baseline().await;
        let renderer = FakeRenderer {
            calls: Arc::new(AtomicUsize::new(0)),
            failed: true,
        };
        let rows = CaptureBaselines::new(store, FakeAgents, renderer)
            .execute(&SpecVersionId::new(test_fixtures::SPEC).unwrap())
            .await
            .unwrap();
        assert_eq!(rows[0].status, latoile_core::VisualBaselineStatus::Failed);
        assert_eq!(rows[0].recovery_action.as_deref(), Some("fix selector"));
    }
}
