//! Channel tests: scripted connections, no processes.

use super::*;
use crate::config::{AgentCommand, AgentTimeouts};
use crate::transport::TurnResult;
use crate::updates::AgentUpdate;
use latoile_core::TriggeredBy;
use latoile_core::ids::{RoleId, SpecVersionId, TaskId};
use latoile_core::ports::{ArchitectureDecision, ArchitecturePackageRequest};
use latoile_core::{ArchitectureOperatingMode, SpecProvenance, SpecVersion};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

type PermissionContextLog = Arc<StdMutex<Vec<(String, Option<PathBuf>)>>>;

/// A scripted connection. `pend` makes prompt wait forever — the
/// cancellation and timeout tests' wedge.
struct FakeConn {
    log: Arc<StdMutex<Vec<String>>>,
    queued: VecDeque<Result<TurnResult, AgentError>>,
    pend: bool,
    dropped: Arc<AtomicBool>,
}

#[allow(clippy::manual_async_fn)] // matching the trait's Send-bound shape
impl Connection for FakeConn {
    fn new_session<'a>(
        &'a mut self,
        _cwd: &'a Path,
    ) -> impl std::future::Future<Output = Result<(), AgentError>> + Send + 'a {
        async move { Ok(()) }
    }
    fn prompt<'a>(
        &'a mut self,
        text: &'a str,
    ) -> impl std::future::Future<Output = Result<TurnResult, AgentError>> + Send + 'a {
        async move {
            self.log.lock().unwrap().push(text.to_string());
            if self.pend {
                std::future::pending::<()>().await;
            }
            self.queued.pop_front().unwrap_or_else(|| {
                Ok(TurnResult {
                    outcome: RunOutcome::Finished,
                    text: "réponse".into(),
                    updates: vec![AgentUpdate::TextChunk("réponse".into())],
                })
            })
        }
    }
    fn cancel(&mut self) -> impl std::future::Future<Output = Result<(), AgentError>> + Send + '_ {
        async move { Ok(()) }
    }
}

impl Drop for FakeConn {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct FakeConnector {
    conns: StdMutex<VecDeque<FakeConn>>,
    spawned: AtomicUsize,
    commands: Arc<StdMutex<Vec<String>>>,
    workspaces: Arc<StdMutex<Vec<PathBuf>>>,
    permission_contexts: PermissionContextLog,
}

impl FakeConnector {
    fn push(&self, conn: FakeConn) {
        self.conns.lock().unwrap().push_back(conn);
    }
}

#[allow(clippy::manual_async_fn)] // matching the trait's Send-bound shape
impl Connector for FakeConnector {
    type Conn = FakeConn;
    fn connect<'a>(
        &'a self,
        command: &'a AgentCommand,
        workspace: &'a Path,
        permissions: PermissionContext,
    ) -> impl std::future::Future<Output = Result<FakeConn, AgentError>> + Send + 'a {
        async move {
            self.commands.lock().unwrap().push(command.program.clone());
            self.workspaces
                .lock()
                .unwrap()
                .push(workspace.to_path_buf());
            self.permission_contexts
                .lock()
                .unwrap()
                .push((permissions.role_id.clone(), permissions.write_root.clone()));
            self.spawned.fetch_add(1, Ordering::SeqCst);
            Ok(self.conns.lock().unwrap().pop_front().unwrap_or(FakeConn {
                log: Arc::new(StdMutex::new(vec![])),
                queued: VecDeque::new(),
                pend: false,
                dropped: Arc::new(AtomicBool::new(false)),
            }))
        }
    }
}

fn fixture() -> (tempfile::TempDir, ChannelConfig) {
    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("project-manager");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "SKILL MANAGER").unwrap();
    let architect = dir.path().join("app-architect-brainstorm");
    for relative in crate::preamble::ARCHITECT_SKILL_FILES {
        let path = architect.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("SKILL ARCHITECT — {relative}")).unwrap();
    }
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-qm",
        "fixture",
    ]);
    let config = ChannelConfig {
        skills_dir: dir.path().to_path_buf(),
        ..ChannelConfig::default()
    };
    (dir, config)
}

fn architecture_session() -> ArchitectureSessionId {
    ArchitectureSessionId::new("as1").unwrap()
}

fn channel(
    config: ChannelConfig,
    connector: FakeConnector,
    root: &Path,
) -> AcpChannel<FakeConnector, RootDirs, ChannelConfig> {
    // The config doubles as the static routing source.
    let routing = config.clone();
    AcpChannel::new(config, connector, RootDirs(root.to_path_buf()), routing)
}

fn project() -> ProjectId {
    ProjectId::new("p1").unwrap()
}

fn run(id: &str) -> Run {
    Run::new(
        RunId::new(id).unwrap(),
        TaskId::new("t1").unwrap(),
        RoleId::new("backend").unwrap(),
        TriggeredBy::Manager,
    )
}

async fn wait_for<C: Connector, D: ProjectDirs>(
    ch: &AcpChannel<C, D, ChannelConfig>,
    run: &RunId,
    want: impl Fn(&RunState) -> bool,
) -> RunState {
    for _ in 0..100 {
        if let Some(state) = ch.run_state(run).await {
            if want(&state) {
                return state;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run never reached the expected state");
}

#[tokio::test]
async fn the_manager_answers_and_the_preamble_heads_the_first_prompt_only() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    let log = Arc::new(StdMutex::new(Vec::new()));
    connector.push(FakeConn {
        log: log.clone(),
        queued: VecDeque::new(),
        pend: false,
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let ch = channel(config, connector, dir.path());

    let reply = ch
        .tell_manager(&project(), "construis la page")
        .await
        .unwrap();
    assert_eq!(reply.content, "réponse");
    assert_eq!(reply.actions, None);

    ch.tell_manager(&project(), "et le formulaire ?")
        .await
        .unwrap();
    let prompts = log.lock().unwrap();
    assert!(prompts[0].starts_with("SKILL MANAGER\n\n---\n\n"));
    assert_eq!(prompts[1], "et le formulaire ?", "no preamble twice");
}

#[tokio::test]
async fn the_architect_keeps_one_socratic_session_and_receives_its_skill() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    let log = Arc::new(StdMutex::new(Vec::new()));
    connector.push(FakeConn {
        log: log.clone(),
        queued: VecDeque::from([
            Ok(TurnResult {
                outcome: RunOutcome::Finished,
                text: "question".into(),
                updates: vec![],
            }),
            Ok(TurnResult {
                outcome: RunOutcome::Finished,
                text: "question suivante".into(),
                updates: vec![],
            }),
            Ok(TurnResult {
                outcome: RunOutcome::Finished,
                text: "question imposée".into(),
                updates: vec![],
            }),
            Ok(TurnResult {
                outcome: RunOutcome::Finished,
                text: "contrat réparé".into(),
                updates: vec![],
            }),
        ]),
        pend: false,
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let ch = channel(config, connector, dir.path());

    ch.start_architecture(&project(), &architecture_session(), "Un outil agentique")
        .await
        .unwrap();
    ch.retry_architecture_question(&project(), &architecture_session())
        .await
        .unwrap();
    ch.continue_architecture(&project(), &architecture_session(), "Une équipe produit")
        .await
        .unwrap();
    ch.retry_architecture_contract(
        &project(),
        &architecture_session(),
        ArchitecturePhase::Requirements,
    )
    .await
    .unwrap();

    let prompts = log.lock().unwrap();
    assert!(prompts[0].contains("PINNED SKILL BUNDLE"));
    assert!(prompts[0].contains("references/ui-ux-design.md"));
    assert!(prompts[0].contains("DISCOVERY-ONLY CONTRACT"));
    assert!(prompts[0].contains("kind MUST be `question`"));
    assert!(prompts[1].starts_with("DISCOVERY GUARD\n"));
    assert!(prompts[2].starts_with("OWNER ANSWER\n"));
    assert!(prompts[3].starts_with("CONTRACT REPAIR\n"));
    assert!(prompts[3].contains("persisted phase is `requirements`"));
    assert!(prompts[3].contains("requirements or ux_discovery"));
    assert_eq!(ch.connector.spawned.load(Ordering::SeqCst), 1);
}

struct PackageConn {
    workspace: PathBuf,
    log: Arc<StdMutex<Vec<String>>>,
    escape: bool,
    invalid_gallery_turns: usize,
    turns: usize,
}

impl Connection for PackageConn {
    async fn new_session(&mut self, cwd: &Path) -> Result<(), AgentError> {
        assert_eq!(cwd, self.workspace);
        Ok(())
    }

    async fn prompt(&mut self, text: &str) -> Result<TurnResult, AgentError> {
        self.log.lock().unwrap().push(text.to_string());
        if self.escape {
            std::fs::create_dir_all(self.workspace.join("src")).unwrap();
            std::fs::write(self.workspace.join("src/forbidden.rs"), "fn escaped() {}\n").unwrap();
            return Ok(TurnResult {
                outcome: RunOutcome::Finished,
                text: "I also changed production source".into(),
                updates: vec![],
            });
        }
        let invalid_gallery = self.turns < self.invalid_gallery_turns;
        self.turns += 1;
        let root = self.workspace.join("design/v0001-as1/");
        std::fs::create_dir_all(root.join("adrs")).unwrap();
        std::fs::create_dir_all(root.join("mockups")).unwrap();
        let tokens = b"# Design tokens\n\n- color-accent: #7c5cff\n";
        std::fs::write(root.join("design-tokens.md"), tokens).unwrap();
        let token_digest = format!("{:x}", Sha256::digest(tokens));
        // Model-authored provenance is deliberately wrong: the adapter must
        // bind the server-owned digest before validation and commit.
        let skill_digest = "model-supplied-wrong-digest";
        let manifest = format!(
            "```latoile-package\n{{\"schema_version\":2,\"skill_digest\":\"{skill_digest}\",\"operating_mode\":\"greenfield\",\"deliverables\":[{{\"path\":\"package-manifest.md\",\"kind\":\"manifest\"}},{{\"path\":\"architecture-spec.md\",\"kind\":\"document\"}},{{\"path\":\"domain-model.md\",\"kind\":\"document\"}},{{\"path\":\"data-model.md\",\"kind\":\"document\"}},{{\"path\":\"api-contract.md\",\"kind\":\"document\"}},{{\"path\":\"architecture-blueprint.md\",\"kind\":\"document\"}},{{\"path\":\"component-specification.md\",\"kind\":\"document\"}},{{\"path\":\"stack-decisions.md\",\"kind\":\"document\"}},{{\"path\":\"architecture-contract.md\",\"kind\":\"document\"}},{{\"path\":\"guardian-checklist.md\",\"kind\":\"document\"}},{{\"path\":\"user-flows.md\",\"kind\":\"document\"}},{{\"path\":\"screen-inventory.md\",\"kind\":\"document\"}},{{\"path\":\"design-tokens.md\",\"kind\":\"tokens\"}},{{\"path\":\"gallery.html\",\"kind\":\"gallery\"}},{{\"path\":\"adrs/ADR-001-boundary.md\",\"kind\":\"decision\"}},{{\"path\":\"mockups/home.html\",\"kind\":\"mockup\"}}],\"p0_scenarios\":[{{\"comparison_id\":\"P0-home\",\"screen\":\"Home\",\"state\":\"default\",\"locale\":\"fr-FR\",\"theme\":\"light\",\"route\":\"/\",\"fixture\":\"synthetic-default\",\"readiness_selector\":\"main\",\"stable_selectors\":[\"main\"],\"allowed_masks\":[],\"viewport\":{{\"width\":390,\"height\":844,\"device_scale_factor_milli\":1000}},\"mockup\":\"mockups/home.html\"}}]}}\n```\n"
        );
        std::fs::write(root.join("package-manifest.md"), manifest).unwrap();
        for file in [
            "architecture-spec.md",
            "domain-model.md",
            "data-model.md",
            "api-contract.md",
            "architecture-blueprint.md",
            "component-specification.md",
            "stack-decisions.md",
            "architecture-contract.md",
            "guardian-checklist.md",
            "user-flows.md",
        ] {
            std::fs::write(
                root.join(file),
                format!("# {file}\n\nDecision-backed content.\n"),
            )
            .unwrap();
        }
        std::fs::write(
            root.join("screen-inventory.md"),
            "# Screens\n\n- P0-home — Home\n",
        )
        .unwrap();
        std::fs::write(root.join("adrs/ADR-001-boundary.md"), "# ADR-001\n").unwrap();
        std::fs::write(
            root.join("mockups/home.html"),
            format!(
                "<!doctype html><html data-latoile-token-digest=\"{token_digest}\" data-latoile-comparison-id=\"P0-home\" data-latoile-screen=\"Home\" data-latoile-state=\"default\" data-latoile-locale=\"fr-FR\" data-latoile-theme=\"light\" data-latoile-route=\"/\" data-latoile-fixture=\"synthetic-default\" data-latoile-viewport=\"390x844@1000\"><body><main>Home</main></body></html>"
            ),
        )
        .unwrap();
        let gallery = if invalid_gallery {
            "<!doctype html><html><body><a href=\"mockups/home.html\">Missing token binding</a></body></html>".into()
        } else {
            format!(
                "<!doctype html><html data-latoile-token-digest=\"{token_digest}\"><body><a href=\"mockups/home.html\">Home</a></body></html>"
            )
        };
        std::fs::write(root.join("gallery.html"), gallery).unwrap();
        Ok(TurnResult {
            outcome: RunOutcome::Finished,
            text: "Verified package ready".into(),
            updates: vec![],
        })
    }

    async fn cancel(&mut self) -> Result<(), AgentError> {
        Ok(())
    }
}

#[derive(Default)]
struct PackageConnector {
    log: Arc<StdMutex<Vec<String>>>,
    contexts: PermissionContextLog,
    escape: bool,
    invalid_gallery_turns: usize,
}

impl Connector for PackageConnector {
    type Conn = PackageConn;

    async fn connect(
        &self,
        _command: &AgentCommand,
        workspace: &Path,
        permissions: PermissionContext,
    ) -> Result<Self::Conn, AgentError> {
        self.contexts
            .lock()
            .unwrap()
            .push((permissions.role_id, permissions.write_root));
        Ok(PackageConn {
            workspace: workspace.to_path_buf(),
            log: self.log.clone(),
            escape: self.escape,
            invalid_gallery_turns: self.invalid_gallery_turns,
            turns: 0,
        })
    }
}

#[tokio::test]
async fn the_acp_adapter_rejects_and_does_not_integrate_an_escape() {
    let (dir, config) = fixture();
    let bundle = Preambles::new(config.skills_dir.clone())
        .architect_bundle()
        .unwrap();
    let base = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let base = String::from_utf8(base.stdout).unwrap().trim().to_string();
    let connector = PackageConnector {
        escape: true,
        ..PackageConnector::default()
    };
    let routing = config.clone();
    let channel: AcpChannel<PackageConnector, RootDirs, ChannelConfig> = AcpChannel::new(
        config,
        connector,
        RootDirs(dir.path().to_path_buf()),
        routing,
    );
    let result = channel
        .generate_architecture_package(
            &project(),
            &architecture_session(),
            &ArchitecturePackageRequest {
                design_dir: "design/v0001-as1/".into(),
                brief: "Build one Home screen only.".into(),
                skill_digest: bundle.digest,
                operating_mode: ArchitectureOperatingMode::Greenfield,
                requested_locale: "fr-FR".into(),
                decisions: vec![ArchitectureDecision {
                    sequence: 1,
                    prompt: "Who?".into(),
                    answer: "Owner".into(),
                }],
            },
        )
        .await;
    assert!(
        result
            .unwrap_err()
            .0
            .contains("outside the static package scope")
    );
    assert!(!dir.path().join("src/forbidden.rs").exists());
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), base);
}

#[tokio::test]
async fn the_acp_adapter_generates_only_a_complete_pinned_package() {
    let (dir, config) = fixture();
    let bundle = Preambles::new(config.skills_dir.clone())
        .architect_bundle()
        .unwrap();
    let connector = PackageConnector {
        invalid_gallery_turns: 1,
        ..PackageConnector::default()
    };
    let log = connector.log.clone();
    let contexts = connector.contexts.clone();
    let routing = config.clone();
    let channel: AcpChannel<PackageConnector, RootDirs, ChannelConfig> = AcpChannel::new(
        config,
        connector,
        RootDirs(dir.path().to_path_buf()),
        routing,
    );

    let generated = channel
        .generate_architecture_package(
            &project(),
            &architecture_session(),
            &ArchitecturePackageRequest {
                design_dir: "design/v0001-as1/".into(),
                brief: "Build one Home screen only.".into(),
                skill_digest: bundle.digest.clone(),
                operating_mode: ArchitectureOperatingMode::Greenfield,
                requested_locale: "fr-FR".into(),
                decisions: vec![ArchitectureDecision {
                    sequence: 1,
                    prompt: "Who is the user?".into(),
                    answer: "A product team".into(),
                }],
            },
        )
        .await
        .unwrap();

    assert_eq!(generated.evidence.package_digest.len(), 64);
    assert!(
        generated
            .evidence
            .changed_files
            .iter()
            .all(|path| path.starts_with("design/v0001-as1/"))
    );
    assert!(
        dir.path()
            .join("design/v0001-as1/mockups/home.html")
            .is_file()
    );
    let (prompt, repair_prompt) = {
        let prompts = log.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        (prompts[0].clone(), prompts[1].clone())
    };
    assert!(prompt.contains("references/brainstorming-method.md"));
    assert!(prompt.contains("assets/templates/arch-spec-template.md"));
    assert!(prompt.contains(&bundle.digest));
    assert!(prompt.contains("__LATOILE_SERVER_BOUND__"));
    assert!(prompt.contains("Owner package locale: fr-FR"));
    assert!(prompt.contains("Every scenario `locale` MUST exactly equal"));
    assert!(prompt.contains("ORIGINAL OWNER BRIEF — PRIMARY SCOPE AUTHORITY"));
    assert!(prompt.contains("Build one Home screen only."));
    assert!(repair_prompt.starts_with("PACKAGE VALIDATION REPAIR 1/2\n"));
    assert!(repair_prompt.contains("does not pin the shared design tokens"));
    let manifest = std::fs::read_to_string(
        dir.path().join("design/v0001-as1/package-manifest.md"),
    )
    .unwrap();
    assert!(manifest.contains(&bundle.digest));
    assert!(manifest.contains("\"operating_mode\": \"greenfield\""));
    assert!(manifest.contains("\"schema_version\": 2"));
    assert!(!manifest.contains("model-supplied-wrong-digest"));
    {
        let contexts = contexts.lock().unwrap();
        assert_eq!(contexts[0].0, "architect_package");
        assert!(
            contexts[0]
                .1
                .as_ref()
                .unwrap()
                .ends_with("design/v0001-as1")
        );
    }

    let mut spec = SpecVersion::new(
        SpecVersionId::new("spec-1").unwrap(),
        project(),
        1,
        "design/v0001-as1/",
        None,
    )
    .unwrap();
    spec.attach_provenance(SpecProvenance {
        architecture_session_id: architecture_session(),
        skill_name: latoile_core::ARCHITECT_SKILL_NAME.into(),
        skill_digest: bundle.digest,
        operating_mode: ArchitectureOperatingMode::Greenfield,
        package_digest: generated.evidence.package_digest,
        manifest_digest: generated.evidence.manifest_digest,
        package_commit_sha: generated.evidence.head_sha,
        package_tree_sha: generated.evidence.tree_sha,
    })
    .unwrap();
    let verified = channel
        .verify_architecture_package(&project(), &spec)
        .await
        .unwrap();
    assert!(verified.valid);
    assert_eq!(verified.scenarios[0].comparison_id, "P0-home");
    assert!(
        channel
            .read_architecture_artifact(&project(), &spec, "gallery.html")
            .await
            .unwrap()
            .contains("mockups/home.html")
    );

    std::fs::write(
        dir.path().join("design/v0001-as1/mockups/home.html"),
        "changed after draft",
    )
    .unwrap();
    let drifted = channel
        .verify_architecture_package(&project(), &spec)
        .await
        .unwrap();
    assert!(!drifted.valid);
    assert_eq!(drifted.findings[0].code, "immutable_package_invalid");
}

#[tokio::test]
async fn architecture_package_validation_repairs_are_bounded() {
    let (dir, config) = fixture();
    let bundle = Preambles::new(config.skills_dir.clone())
        .architect_bundle()
        .unwrap();
    let connector = PackageConnector {
        invalid_gallery_turns: 3,
        ..PackageConnector::default()
    };
    let log = connector.log.clone();
    let routing = config.clone();
    let channel: AcpChannel<PackageConnector, RootDirs, ChannelConfig> = AcpChannel::new(
        config,
        connector,
        RootDirs(dir.path().to_path_buf()),
        routing,
    );

    let error = channel
        .generate_architecture_package(
            &project(),
            &architecture_session(),
            &ArchitecturePackageRequest {
                design_dir: "design/v0001-as1/".into(),
                brief: "Build one Home screen only.".into(),
                skill_digest: bundle.digest,
                operating_mode: ArchitectureOperatingMode::Greenfield,
                requested_locale: "fr-FR".into(),
                decisions: vec![ArchitectureDecision {
                    sequence: 1,
                    prompt: "Who is the user?".into(),
                    answer: "A product team".into(),
                }],
            },
        )
        .await
        .unwrap_err();

    assert!(error.0.contains("does not pin the shared design tokens"));
    let prompts = log.lock().unwrap();
    assert_eq!(prompts.len(), 3);
    assert!(prompts[1].starts_with("PACKAGE VALIDATION REPAIR 1/2\n"));
    assert!(prompts[2].starts_with("PACKAGE VALIDATION REPAIR 2/2\n"));
    assert!(!dir.path().join("design/v0001-as1").exists());
}

#[tokio::test]
async fn a_wedged_manager_is_evicted_and_the_next_message_respawns() {
    let (dir, mut config) = fixture();
    config.timeouts = AgentTimeouts {
        prompt: Duration::from_millis(50),
        ..AgentTimeouts::default()
    };
    let connector = FakeConnector::default();
    let wedged = Arc::new(AtomicBool::new(false));
    connector.push(FakeConn {
        log: Arc::new(StdMutex::new(vec![])),
        queued: VecDeque::new(),
        pend: true,
        dropped: wedged.clone(),
    });
    let ch = channel(config, connector, dir.path());

    let err = ch.tell_manager(&project(), "allo").await;
    assert!(err.is_err(), "a wedged manager must not hang the caller");
    assert!(
        wedged.load(Ordering::SeqCst),
        "the wedged process is killed"
    );

    // Next message: a fresh session answers.
    let reply = ch.tell_manager(&project(), "allo").await.unwrap();
    assert_eq!(reply.content, "réponse");
}

#[tokio::test]
async fn a_run_completes_in_the_background() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    let ch = channel(config, connector, dir.path());
    let r = run("r1");

    let handle = ch
        .start_run(&project(), &r, "implémente le endpoint")
        .await
        .unwrap();
    assert_eq!(handle, "acp:r1");

    let state = wait_for(&ch, &r.id, |s| !matches!(s, RunState::Running)).await;
    assert!(matches!(
        state,
        RunState::Done(RunReport {
            outcome: RunOutcome::Finished,
            ref summary,
            ..
        }) if summary == "réponse"
    ));
}

#[tokio::test]
async fn a_run_that_dies_mid_stream_records_a_failure_not_a_hang() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    connector.push(FakeConn {
        log: Arc::new(StdMutex::new(vec![])),
        queued: VecDeque::from([Err(AgentError::AgentGone)]),
        pend: false,
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let ch = channel(config, connector, dir.path());
    let r = run("r2");

    ch.start_run(&project(), &r, "travaille").await.unwrap();
    let state = wait_for(&ch, &r.id, |s| !matches!(s, RunState::Running)).await;
    assert!(matches!(state, RunState::Failed(_)), "{state:?}");
}

#[tokio::test]
async fn cancelling_a_run_kills_its_process() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    let dropped = Arc::new(AtomicBool::new(false));
    connector.push(FakeConn {
        log: Arc::new(StdMutex::new(vec![])),
        queued: VecDeque::new(),
        pend: true,
        dropped: dropped.clone(),
    });
    let ch = channel(config, connector, dir.path());
    let r = run("r3");

    ch.start_run(&project(), &r, "travaille").await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await; // let it start
    ch.cancel_run(&r.id).await.unwrap();

    assert_eq!(
        ch.run_state(&r.id).await,
        None,
        "a cancelled run leaves the registry"
    );
    for _ in 0..100 {
        if dropped.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the cancelled run's process was not killed");
}

#[tokio::test]
async fn a_run_past_its_time_budget_is_killed() {
    let (dir, mut config) = fixture();
    config.timeouts = AgentTimeouts {
        prompt: Duration::from_millis(50),
        ..AgentTimeouts::default()
    };
    let connector = FakeConnector::default();
    connector.push(FakeConn {
        log: Arc::new(StdMutex::new(vec![])),
        queued: VecDeque::new(),
        pend: true,
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let ch = channel(config, connector, dir.path());
    let r = run("r4");

    ch.start_run(&project(), &r, "travaille").await.unwrap();
    let state = wait_for(&ch, &r.id, |s| !matches!(s, RunState::Running)).await;
    assert_eq!(state, RunState::Failed("timed out".into()));
}

#[tokio::test]
async fn cancelling_an_unknown_run_is_fine() {
    let (dir, config) = fixture();
    let ch = channel(config, FakeConnector::default(), dir.path());
    ch.cancel_run(&RunId::new("ghost").unwrap()).await.unwrap();
}

/// A routing change applies to NEW sessions only: the live manager
/// keeps its provider until evicted.
#[tokio::test]
async fn a_routing_change_applies_after_eviction() {
    use std::sync::RwLock;

    #[derive(Clone)]
    struct Switchable(std::sync::Arc<RwLock<String>>);
    impl crate::channel::RoutingSource for Switchable {
        fn command_for(&self, _role: &str) -> AgentCommand {
            AgentCommand::new(self.0.read().unwrap().clone())
        }
    }

    let (dir, config) = fixture();
    let routing = Switchable(std::sync::Arc::new(RwLock::new("claude-agent-acp".into())));
    let connector = FakeConnector::default();
    let commands = connector.commands.clone();
    let ch: AcpChannel<FakeConnector, RootDirs, _> = AcpChannel::new(
        config,
        connector,
        RootDirs(dir.path().to_path_buf()),
        routing.clone(),
    );

    ch.tell_manager(&project(), "allo").await.unwrap();
    assert_eq!(commands.lock().unwrap().last().unwrap(), "claude-agent-acp");

    // The user switches the manager to codex: the LIVE session does
    // not move…
    *routing.0.write().unwrap() = "codex-acp".into();
    ch.tell_manager(&project(), "encore").await.unwrap();
    assert_eq!(commands.lock().unwrap().last().unwrap(), "claude-agent-acp");

    // …until eviction; the next message spawns under the new provider.
    ch.evict_managers().await;
    ch.tell_manager(&project(), "encore").await.unwrap();
    assert_eq!(commands.lock().unwrap().last().unwrap(), "codex-acp");
}

/// The resolved project directory is what the agent spawns in — this is
/// the E2E bug's regression test.
#[tokio::test]
async fn the_session_starts_in_the_project_directory() {
    let (dir, config) = fixture();
    let project_dir = dir.path().join("checkout");
    std::fs::create_dir_all(&project_dir).unwrap();

    struct FixedDirs(PathBuf);
    impl ProjectDirs for FixedDirs {
        async fn manager_dir(&self, _p: &ProjectId) -> Option<PathBuf> {
            Some(self.0.clone())
        }
        async fn run_dir(&self, _project: &ProjectId, _r: &Run) -> Option<PathBuf> {
            Some(self.0.clone())
        }
    }

    let connector = FakeConnector::default();
    let workspaces = connector.workspaces.clone();
    let routing = config.clone();
    let ch: AcpChannel<FakeConnector, FixedDirs, ChannelConfig> =
        AcpChannel::new(config, connector, FixedDirs(project_dir.clone()), routing);

    ch.tell_manager(&project(), "allo").await.unwrap();
    let run = run("unpersisted-run");
    ch.start_run(&project(), &run, "travaille").await.unwrap();
    assert_eq!(
        workspaces.lock().unwrap().as_slice(),
        &[project_dir.clone(), project_dir]
    );
}

/// A project directory that doesn't exist is a clear error, before any
/// spawn — never a 30-second timeout.
#[tokio::test]
async fn a_nonexistent_project_directory_fails_fast() {
    let (dir, config) = fixture();
    let connector = FakeConnector::default();
    let workspaces = connector.workspaces.clone();
    let missing = dir.path().join("never-created");
    let routing = config.clone();
    let ch = AcpChannel::new(config, connector, RootDirs(missing.clone()), routing);

    let err = ch.tell_manager(&project(), "allo").await.unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
    assert!(err.to_string().contains("never-created"), "{err}");
    assert!(
        workspaces.lock().unwrap().is_empty(),
        "nothing was spawned for a bad cwd"
    );
}
