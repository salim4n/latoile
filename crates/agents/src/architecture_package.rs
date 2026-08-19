//! Isolated Architect package generation. The agent writes in a detached
//! temporary Git worktree, never in the project's live checkout. LaToile
//! validates the complete static package and the exact path-level diff,
//! commits it itself, then integrates only that verified commit by fast
//! forward.
//!
//! This module deliberately keeps generation-time and approval-time package
//! validation in one trust boundary: both paths call the same manifest and
//! digest functions. Splitting those rules into independently evolving
//! modules would reintroduce the exact validation drift this boundary exists
//! to prevent. ACP transport and permission policy remain separate modules.

use crate::config::{AgentCommand, AgentTimeouts};
use crate::error::AgentError;
use crate::preamble::ArchitectSkillBundle;
use crate::transport::{Connection, Connector, PermissionContext};
use crate::updates::RunOutcome;
use latoile_core::ports::{ArchitecturePackageReply, ArchitecturePackageRequest};
use latoile_core::{
    ArchitectureOperatingMode, ArchitecturePackageEvidence, ArchitecturePackageValidation,
    ArchitectureSessionId, ArchitectureValidationFinding, ArchitectureVisualScenario, SpecVersion,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

const REQUIRED_FILES: &[&str] = &[
    "package-manifest.md",
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
    "screen-inventory.md",
    "design-tokens.md",
    "gallery.html",
];

const MAX_PACKAGE_FILES: usize = 100;
const MAX_PACKAGE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_PACKAGE_REPAIR_TURNS: usize = 2;
const SERVER_BOUND_MANIFEST_VALUE: &str = "__LATOILE_SERVER_BOUND__";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackageManifest {
    schema_version: u32,
    skill_digest: String,
    operating_mode: String,
    deliverables: Vec<ManifestDeliverable>,
    p0_scenarios: Vec<P0Scenario>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestDeliverable {
    path: String,
    kind: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct P0Scenario {
    comparison_id: String,
    screen: String,
    state: String,
    locale: String,
    theme: String,
    route: String,
    fixture: String,
    readiness_selector: String,
    stable_selectors: Vec<String>,
    allowed_masks: Vec<String>,
    viewport: ManifestViewport,
    mockup: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestViewport {
    width: u32,
    height: u32,
    device_scale_factor_milli: u32,
}

struct ValidatedPackage {
    package_digest: String,
    manifest_digest: String,
    file_count: u32,
    scenarios: Vec<ArchitectureVisualScenario>,
}

pub async fn detect_operating_mode(dir: &Path) -> Result<ArchitectureOperatingMode, AgentError> {
    let tracked = git_text(dir, &["ls-files"]).await?;
    let has_product_source = tracked.lines().any(|path| {
        let lower = path.to_ascii_lowercase();
        lower.starts_with("src/")
            || lower.starts_with("app/")
            || lower.starts_with("web/")
            || lower.starts_with("crates/")
            || matches!(
                Path::new(&lower).file_name().and_then(|name| name.to_str()),
                Some(
                    "cargo.toml"
                        | "package.json"
                        | "pyproject.toml"
                        | "go.mod"
                        | "pom.xml"
                        | "build.gradle"
                )
            )
            || matches!(
                Path::new(&lower)
                    .extension()
                    .and_then(|value| value.to_str()),
                Some(
                    "rs" | "ts"
                        | "tsx"
                        | "js"
                        | "jsx"
                        | "py"
                        | "go"
                        | "java"
                        | "kt"
                        | "swift"
                        | "cs"
                        | "rb"
                        | "php"
                )
            )
    });
    Ok(if has_product_source {
        ArchitectureOperatingMode::ReverseEngineering
    } else {
        ArchitectureOperatingMode::Greenfield
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn generate<C: Connector>(
    connector: &C,
    command: &AgentCommand,
    project_dir: &Path,
    session: &ArchitectureSessionId,
    request: &ArchitecturePackageRequest,
    bundle: &ArchitectSkillBundle,
    permissions: PermissionContext,
    timeouts: AgentTimeouts,
) -> Result<ArchitecturePackageReply, AgentError> {
    validate_request(request, bundle)?;
    let current_mode = detect_operating_mode(project_dir).await?;
    if current_mode != request.operating_mode {
        return Err(AgentError::Prompt(
            "the project archetype changed after discovery; restart architecture".into(),
        ));
    }
    let base_sha = git_text(project_dir, &["rev-parse", "HEAD"]).await?;
    let temp = tempfile::Builder::new()
        .prefix("latoile-architect-")
        .tempdir()
        .map_err(|error| AgentError::Prompt(format!("creating isolated worktree: {error}")))?;
    let worktree = temp.path().join("checkout");
    let worktree_arg = path_text(&worktree)?;
    git_ok(
        project_dir,
        &["worktree", "add", "--detach", &worktree_arg, &base_sha],
    )
    .await?;

    let result = generate_in_worktree(
        connector,
        command,
        project_dir,
        &worktree,
        session,
        request,
        bundle,
        permissions,
        timeouts,
        &base_sha,
    )
    .await;

    // The target is an exact temporary path created above. Removing it is
    // cleanup of our own detached worktree, not user data.
    let _ = git_ok(
        project_dir,
        &["worktree", "remove", "--force", &worktree_arg],
    )
    .await;
    result
}

#[allow(clippy::too_many_arguments)]
async fn generate_in_worktree<C: Connector>(
    connector: &C,
    command: &AgentCommand,
    project_dir: &Path,
    worktree: &Path,
    session: &ArchitectureSessionId,
    request: &ArchitecturePackageRequest,
    bundle: &ArchitectSkillBundle,
    mut permissions: PermissionContext,
    timeouts: AgentTimeouts,
    base_sha: &str,
) -> Result<ArchitecturePackageReply, AgentError> {
    let package_root = worktree.join(request.design_dir.trim_end_matches('/'));
    permissions.role_id = "architect_package".into();
    permissions.write_root = Some(package_root.clone());
    let mut conn = connector.connect(command, worktree, permissions).await?;
    conn.new_session(worktree).await?;
    let prompt = package_prompt(bundle, request);
    let mut turn = tokio::time::timeout(timeouts.prompt, conn.prompt(&prompt))
        .await
        .map_err(|_| {
            AgentError::Timeout(format!(
                "architecture package (session {}, cwd {})",
                session.as_str(),
                worktree.display()
            ))
        })??;
    let mut repairs = 0usize;
    let validated = loop {
        if turn.outcome != RunOutcome::Finished {
            return Err(AgentError::Prompt(
                "the Architect did not finish the package turn".into(),
            ));
        }
        let changed_files = changed_files(worktree).await?;
        // Confinement is never repairable: any path outside the selected
        // package root fails immediately rather than being shown back to the
        // model as a softer content issue.
        validate_changed_paths(&changed_files, &request.design_dir)?;
        let validation = bind_manifest_provenance(
            &package_root,
            &request.skill_digest,
            request.operating_mode,
        )
        .and_then(|_| {
            validate_package(
                &package_root,
                &request.skill_digest,
                request.operating_mode,
            )
        })
        .and_then(|validated| {
            validate_requested_locale(validated, &request.requested_locale)
        });
        match validation {
            Ok(validated) => break validated,
            Err(error) if repairs < MAX_PACKAGE_REPAIR_TURNS => {
                repairs += 1;
                let repair_prompt = format!(
                    "PACKAGE VALIDATION REPAIR {repairs}/{MAX_PACKAGE_REPAIR_TURNS}\nNo new owner answer was supplied. LaToile rejected the package with this bounded validator result:\n{error}\n\nFix only that validation defect and any directly inconsistent package metadata under `{}`. Preserve every durable owner decision, the server-bound provenance, the single P0 contract and all already-valid files. Do not execute commands or write outside the package directory. Finish the repair turn when the package is complete; LaToile will rebind provenance and re-run every validator.",
                    request.design_dir,
                );
                turn = tokio::time::timeout(timeouts.prompt, conn.prompt(&repair_prompt))
                    .await
                    .map_err(|_| {
                        AgentError::Timeout(format!(
                            "architecture package repair (session {}, cwd {})",
                            session.as_str(),
                            worktree.display()
                        ))
                    })??;
            }
            Err(error) => return Err(error),
        }
    };

    git_ok(worktree, &["add", "--", &request.design_dir]).await?;
    git_ok(
        worktree,
        &[
            "-c",
            "user.name=LaToile Architect",
            "-c",
            "user.email=architect@latoile.local",
            "commit",
            "-m",
            "docs: generate architecture package",
        ],
    )
    .await?;
    let head_sha = git_text(worktree, &["rev-parse", "HEAD"]).await?;
    let tree_sha = git_text(worktree, &["rev-parse", "HEAD^{tree}"]).await?;
    let committed_paths = git_text(worktree, &["diff", "--name-only", base_sha, &head_sha])
        .await?
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    validate_changed_paths(&committed_paths, &request.design_dir)?;
    validate_committed_bytes(worktree, &package_root, &request.design_dir, &head_sha).await?;
    let diff_stat = git_text(worktree, &["diff", "--stat", base_sha, &head_sha]).await?;

    let live_head = git_text(project_dir, &["rev-parse", "HEAD"]).await?;
    if live_head != base_sha {
        return Err(AgentError::Prompt(
            "the project HEAD changed during architecture generation; package was not integrated"
                .into(),
        ));
    }
    git_ok(project_dir, &["merge", "--ff-only", &head_sha]).await?;
    let integrated = git_text(project_dir, &["rev-parse", "HEAD"]).await?;
    if integrated != head_sha {
        return Err(AgentError::Prompt(
            "the verified architecture commit was not integrated exactly".into(),
        ));
    }

    Ok(ArchitecturePackageReply {
        evidence: ArchitecturePackageEvidence {
            design_dir: request.design_dir.clone(),
            base_sha: base_sha.to_string(),
            head_sha,
            tree_sha,
            package_digest: validated.package_digest,
            manifest_digest: validated.manifest_digest,
            changed_files: committed_paths,
            diff_stat: truncate(diff_stat, 32 * 1024),
        },
        summary: truncate(turn.text.trim().to_string(), 16 * 1024),
    })
}

async fn validate_committed_bytes(
    worktree: &Path,
    package_root: &Path,
    design_dir: &str,
    commit: &str,
) -> Result<(), AgentError> {
    for path in package_files(package_root)? {
        let relative = path
            .strip_prefix(package_root)
            .map_err(|_| AgentError::Prompt("package path escaped root".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        let object = format!("{commit}:{design_dir}{relative}");
        let committed = git_bytes(worktree, &["show", &object]).await?;
        let validated = std::fs::read(&path)
            .map_err(|error| AgentError::Prompt(format!("reading package artifact: {error}")))?;
        if committed != validated {
            return Err(AgentError::Prompt(format!(
                "Git filters changed architecture artifact bytes at {design_dir}{relative}"
            )));
        }
    }
    Ok(())
}

fn validate_request(
    request: &ArchitecturePackageRequest,
    bundle: &ArchitectSkillBundle,
) -> Result<(), AgentError> {
    let path = Path::new(&request.design_dir);
    if request.skill_digest != bundle.digest {
        return Err(AgentError::Prompt(
            "the pinned Architect skill digest changed; restart discovery".into(),
        ));
    }
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || !request.design_dir.starts_with("design/v")
        || !request.design_dir.ends_with('/')
    {
        return Err(AgentError::Prompt(
            "architecture package directory must be a versioned path under design/".into(),
        ));
    }
    if request.decisions.is_empty()
        || request
            .decisions
            .iter()
            .any(|decision| decision.prompt.trim().is_empty() || decision.answer.trim().is_empty())
    {
        return Err(AgentError::Prompt(
            "architecture package generation needs durable owner decisions".into(),
        ));
    }
    if request.brief.trim().is_empty() {
        return Err(AgentError::Prompt(
            "architecture package generation needs the original owner brief".into(),
        ));
    }
    if !matches!(request.requested_locale.as_str(), "en-US" | "fr-FR") {
        return Err(AgentError::Prompt(
            "architecture package locale must be en-US or fr-FR".into(),
        ));
    }
    Ok(())
}

fn package_prompt(bundle: &ArchitectSkillBundle, request: &ArchitecturePackageRequest) -> String {
    let decisions = request
        .decisions
        .iter()
        .map(|decision| {
            format!(
                "{}. QUESTION: {}\n   OWNER ANSWER: {}",
                decision.sequence, decision.prompt, decision.answer
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "{}\n\n---\n\nPACKAGE-ONLY AUTHORITY\nOperating mode: {}\nOwner package locale: {}\nPinned skill SHA-256: {}\nWrite ONLY under `{}`. Do not execute commands. Do not modify source, configuration, scripts, dependencies or files outside that directory. Produce specifications, Mermaid diagrams and self-contained static HTML only. Use the owner package locale for all package prose and visible mockup copy; never infer the language from the skill bundle.\n\nORIGINAL OWNER BRIEF — PRIMARY SCOPE AUTHORITY\n{}\n\nDURABLE OWNER DECISIONS — CLARIFY THE BRIEF, NEVER EXPAND IT\n{}\n\nDo not invent additional screens, states, routes, locales, viewports, actors, integrations or P0 scenarios beyond the original brief and explicit owner answers. Explicit counts and exclusions in the brief are hard constraints.\n\nMANDATORY PACKAGE CONTRACT\nCreate every file below:\n- package-manifest.md\n- architecture-spec.md\n- domain-model.md\n- data-model.md\n- api-contract.md\n- architecture-blueprint.md\n- component-specification.md\n- stack-decisions.md\n- architecture-contract.md\n- guardian-checklist.md\n- user-flows.md\n- screen-inventory.md\n- design-tokens.md\n- gallery.html\n- adrs/ADR-001-*.md (at least one ADR)\n- mockups/<scenario>.html (one self-contained page for every P0 scenario)\n\n`package-manifest.md` MUST contain exactly one fenced `latoile-package` JSON object with schema_version 2, the pinned skill_digest and operating_mode. Its `deliverables` array must enumerate EVERY package file exactly once as `{{path, kind}}`. Its non-empty `p0_scenarios` array must define each visual contract as `{{comparison_id, screen, state, locale, theme, route, fixture, readiness_selector, stable_selectors, allowed_masks, viewport: {{width, height, device_scale_factor_milli}}, mockup}}`. Every scenario `locale` MUST exactly equal the owner package locale shown above. Theme is light or dark; route starts with `/`; fixture names synthetic data only; readiness_selector must identify the deterministic ready state; stable_selectors must uniquely identify measured elements; allowed_masks is an explicit subset of stable_selectors and may be empty. Comparison ids are stable, unique, at most 128 characters, and use only ASCII letters, digits, `.`, `-`, or `_`; viewport values are explicit. Every comparison_id must appear in screen-inventory.md. Gallery must link every P0 mockup. Each mockup root must pin its comparison id, screen, state, locale, theme, route, fixture and viewport with `data-latoile-comparison-id`, `data-latoile-screen`, `data-latoile-state`, `data-latoile-locale`, `data-latoile-theme`, `data-latoile-route`, `data-latoile-fixture`, and `data-latoile-viewport=\"<width>x<height>@<device_scale_factor_milli>\"`. Compute SHA-256 of the exact `design-tokens.md` bytes and include `data-latoile-token-digest=\"<digest>\"` on the root element of gallery.html and every mockup. No scripts, event handlers, forms, frames, external assets or network URLs. Finish with a concise summary; LaToile validates and commits the package.",
        bundle.render(),
        request.operating_mode.as_str(),
        request.requested_locale,
        request.skill_digest,
        request.design_dir,
        request.brief,
        decisions,
    );
    prompt.replace(
        "`package-manifest.md` MUST contain exactly one fenced `latoile-package` JSON object with schema_version 2, the pinned skill_digest and operating_mode.",
        &format!(
            "`package-manifest.md` MUST contain exactly one fenced `latoile-package` JSON object with schema_version 2. Set both `skill_digest` and `operating_mode` to the exact string `{SERVER_BOUND_MANIFEST_VALUE}`; LaToile replaces these server-owned provenance fields with the pinned values before validation and commit."
        ),
    )
}

fn validate_requested_locale(
    validated: ValidatedPackage,
    requested_locale: &str,
) -> Result<ValidatedPackage, AgentError> {
    if validated
        .scenarios
        .iter()
        .any(|scenario| scenario.locale != requested_locale)
    {
        return Err(AgentError::Prompt(format!(
            "every P0 scenario locale must equal the owner-selected {requested_locale} locale"
        )));
    }
    Ok(validated)
}

async fn changed_files(worktree: &Path) -> Result<Vec<String>, AgentError> {
    let raw = git_text(
        worktree,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .await?;
    Ok(raw
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|path| path.trim_matches('"').to_string())
        .collect())
}

fn validate_changed_paths(paths: &[String], design_dir: &str) -> Result<(), AgentError> {
    if paths.is_empty() {
        return Err(AgentError::Prompt(
            "the Architect produced no package files".into(),
        ));
    }
    if paths.len() > MAX_PACKAGE_FILES {
        return Err(AgentError::Prompt(
            "the Architect package exceeded the 100-file evidence bound".into(),
        ));
    }
    for path in paths {
        let candidate = Path::new(path);
        let extension = candidate.extension().and_then(|value| value.to_str());
        if !path.starts_with(design_dir)
            || candidate.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
            || !matches!(extension, Some("md" | "html"))
        {
            return Err(AgentError::Prompt(format!(
                "rejected Architect mutation outside the static package scope: {path}"
            )));
        }
    }
    Ok(())
}

fn bind_manifest_provenance(
    root: &Path,
    skill_digest: &str,
    operating_mode: ArchitectureOperatingMode,
) -> Result<(), AgentError> {
    let path = root.join("package-manifest.md");
    require_regular_file(&path, "package-manifest.md")?;
    let bytes = std::fs::read(&path)
        .map_err(|error| AgentError::Prompt(format!("reading package manifest: {error}")))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| AgentError::Prompt("package-manifest.md is not valid UTF-8".into()))?;
    let raw = fenced_block(&text, "latoile-package").ok_or_else(|| {
        AgentError::Prompt("package-manifest.md is missing the latoile-package contract".into())
    })?;
    let mut manifest: PackageManifest = serde_json::from_str(raw)
        .map_err(|error| AgentError::Prompt(format!("invalid latoile-package manifest: {error}")))?;
    manifest.schema_version = 2;
    manifest.skill_digest = skill_digest.to_string();
    manifest.operating_mode = operating_mode.as_str().to_string();
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| AgentError::Prompt(format!("serializing package manifest: {error}")))?;
    std::fs::write(&path, format!("```latoile-package\n{json}\n```\n"))
        .map_err(|error| AgentError::Prompt(format!("binding package manifest: {error}")))
}

fn validate_package(
    root: &Path,
    expected_skill_digest: &str,
    expected_mode: ArchitectureOperatingMode,
) -> Result<ValidatedPackage, AgentError> {
    for required in REQUIRED_FILES {
        require_regular_file(&root.join(required), required)?;
    }
    let adrs = regular_files(&root.join("adrs"), "md")?;
    if adrs.is_empty() {
        return Err(AgentError::Prompt(
            "the architecture package needs at least one ADR".into(),
        ));
    }
    let mockups = regular_files(&root.join("mockups"), "html")?;
    if mockups.is_empty() {
        return Err(AgentError::Prompt(
            "the architecture package needs at least one P0 HTML mockup".into(),
        ));
    }

    let manifest_bytes = std::fs::read(root.join("package-manifest.md"))
        .map_err(|error| AgentError::Prompt(format!("reading package manifest: {error}")))?;
    let manifest_digest = sha256(&manifest_bytes);
    let manifest_text = String::from_utf8(manifest_bytes)
        .map_err(|_| AgentError::Prompt("package-manifest.md is not valid UTF-8".into()))?;
    let manifest_raw = fenced_block(&manifest_text, "latoile-package").ok_or_else(|| {
        AgentError::Prompt("package-manifest.md is missing the latoile-package contract".into())
    })?;
    let manifest: PackageManifest = serde_json::from_str(manifest_raw).map_err(|error| {
        AgentError::Prompt(format!("invalid latoile-package manifest: {error}"))
    })?;
    validate_manifest_header(&manifest, expected_skill_digest, expected_mode)?;

    let package_files = package_files(root)?;
    let actual_paths = package_files
        .iter()
        .map(|path| {
            path.strip_prefix(root)
                .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                .map_err(|_| AgentError::Prompt("package path escaped root".into()))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut declared_paths = BTreeSet::new();
    for deliverable in &manifest.deliverables {
        let path = Path::new(&deliverable.path);
        if deliverable.kind.trim().is_empty()
            || deliverable.path.trim().is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
            || !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("md" | "html")
            )
            || !declared_paths.insert(deliverable.path.clone())
        {
            return Err(AgentError::Prompt(
                "manifest deliverables must be unique safe .md/.html package paths with a kind"
                    .into(),
            ));
        }
    }
    if declared_paths != actual_paths {
        return Err(AgentError::Prompt(
            "manifest deliverables do not exactly match every package file".into(),
        ));
    }

    let inventory = read_bounded(&root.join("screen-inventory.md"))?;
    let gallery = read_bounded(&root.join("gallery.html"))?;
    let mut comparison_ids = BTreeSet::new();
    let mut scenarios = Vec::new();
    for scenario in &manifest.p0_scenarios {
        let mockup_path = Path::new(&scenario.mockup);
        let stable_selectors = scenario.stable_selectors.iter().collect::<BTreeSet<_>>();
        if !comparison_id_safe(&scenario.comparison_id) {
            return Err(AgentError::Prompt(
                "P0 comparison_id must use at most 128 ASCII letters, digits, dots, dashes or underscores"
                    .into(),
            ));
        }
        if scenario.screen.trim().is_empty()
            || scenario.screen.len() > 128
            || scenario.state.trim().is_empty()
            || scenario.state.len() > 128
            || scenario.locale.trim().is_empty()
            || scenario.locale.len() > 35
            || !matches!(scenario.theme.as_str(), "light" | "dark")
            || !scenario.route.starts_with('/')
            || scenario.route.starts_with("//")
            || scenario.route.len() > 256
            || scenario.route.chars().any(char::is_control)
            || scenario.fixture.trim().is_empty()
            || scenario.fixture.len() > 128
            || !scenario.fixture.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
            || !selector_contract_safe(&scenario.readiness_selector)
            || scenario.stable_selectors.is_empty()
            || scenario.stable_selectors.len() > 32
            || stable_selectors.len() != scenario.stable_selectors.len()
            || scenario
                .stable_selectors
                .iter()
                .any(|selector| !selector_contract_safe(selector))
            || scenario.allowed_masks.len() > 16
            || scenario.allowed_masks.iter().any(|selector| {
                !selector_contract_safe(selector) || !stable_selectors.contains(selector)
            })
            || scenario.viewport.width == 0
            || scenario.viewport.width > 4096
            || scenario.viewport.height == 0
            || scenario.viewport.height > 4096
            || !(500..=4000).contains(&scenario.viewport.device_scale_factor_milli)
            || !comparison_ids.insert(scenario.comparison_id.clone())
            || !scenario.mockup.starts_with("mockups/")
            || mockup_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
            || mockup_path.extension().and_then(|value| value.to_str()) != Some("html")
        {
            return Err(AgentError::Prompt(
                "every P0 scenario needs an id, screen and mockups/*.html path".into(),
            ));
        }
        require_regular_file(&root.join(&scenario.mockup), &scenario.mockup)?;
        if !inventory.contains(&scenario.comparison_id) || !gallery.contains(&scenario.mockup) {
            return Err(AgentError::Prompt(format!(
                "P0 scenario {} is not traceable through inventory and gallery",
                scenario.comparison_id
            )));
        }
        let mockup_text = read_bounded(&root.join(&scenario.mockup))?;
        let markers = [
            format!("data-latoile-comparison-id=\"{}\"", scenario.comparison_id),
            format!("data-latoile-screen=\"{}\"", scenario.screen),
            format!("data-latoile-state=\"{}\"", scenario.state),
            format!("data-latoile-locale=\"{}\"", scenario.locale),
            format!("data-latoile-theme=\"{}\"", scenario.theme),
            format!("data-latoile-route=\"{}\"", scenario.route),
            format!("data-latoile-fixture=\"{}\"", scenario.fixture),
            format!(
                "data-latoile-viewport=\"{}x{}@{}\"",
                scenario.viewport.width,
                scenario.viewport.height,
                scenario.viewport.device_scale_factor_milli
            ),
        ];
        if markers.iter().any(|marker| !mockup_text.contains(marker)) {
            return Err(AgentError::Prompt(format!(
                "P0 mockup {} does not pin its comparison metadata",
                scenario.mockup
            )));
        }
        scenarios.push(ArchitectureVisualScenario {
            comparison_id: scenario.comparison_id.clone(),
            screen: scenario.screen.clone(),
            state: scenario.state.clone(),
            locale: scenario.locale.clone(),
            theme: scenario.theme.clone(),
            route: scenario.route.clone(),
            fixture: scenario.fixture.clone(),
            readiness_selector: scenario.readiness_selector.clone(),
            stable_selectors: scenario.stable_selectors.clone(),
            allowed_masks: scenario.allowed_masks.clone(),
            viewport_width: scenario.viewport.width,
            viewport_height: scenario.viewport.height,
            device_scale_factor_milli: scenario.viewport.device_scale_factor_milli,
            mockup: scenario.mockup.clone(),
        });
    }
    for mockup in &mockups {
        let relative = mockup
            .strip_prefix(root)
            .map_err(|_| AgentError::Prompt("mockup escaped package root".into()))?
            .to_string_lossy();
        if !manifest
            .p0_scenarios
            .iter()
            .any(|scenario| scenario.mockup == relative)
        {
            return Err(AgentError::Prompt(format!(
                "mockup {relative} has no declared P0 scenario"
            )));
        }
    }

    let tokens = std::fs::read(root.join("design-tokens.md"))
        .map_err(|error| AgentError::Prompt(format!("reading design tokens: {error}")))?;
    let token_digest = sha256(&tokens);
    for html in package_files
        .iter()
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("html"))
    {
        let text = read_bounded(html)?;
        let lowered = text.to_ascii_lowercase();
        if !text.contains(&format!("data-latoile-token-digest=\"{token_digest}\"")) {
            return Err(AgentError::Prompt(format!(
                "HTML artifact {} does not pin the shared design tokens",
                html.display()
            )));
        }
        if !html_is_self_contained(&lowered) {
            return Err(AgentError::Prompt(format!(
                "HTML artifact {} contains forbidden external or active content",
                html.display()
            )));
        }
    }

    let package_digest = digest_tree(root)?;
    Ok(ValidatedPackage {
        package_digest,
        manifest_digest,
        file_count: u32::try_from(package_files.len())
            .map_err(|_| AgentError::Prompt("package file count overflowed".into()))?,
        scenarios,
    })
}

fn validate_manifest_header(
    manifest: &PackageManifest,
    expected_skill_digest: &str,
    expected_mode: ArchitectureOperatingMode,
) -> Result<(), AgentError> {
    if manifest.schema_version != 2 {
        return Err(AgentError::Prompt(
            "package manifest schema_version must be 2".into(),
        ));
    }
    if manifest.skill_digest != expected_skill_digest {
        return Err(AgentError::Prompt(
            "package manifest skill_digest does not match the server-pinned Architect bundle"
                .into(),
        ));
    }
    if manifest.operating_mode != expected_mode.as_str() {
        return Err(AgentError::Prompt(
            "package manifest operating_mode does not match the server-pinned project mode"
                .into(),
        ));
    }
    if manifest.deliverables.is_empty() {
        return Err(AgentError::Prompt(
            "package manifest deliverables must not be empty".into(),
        ));
    }
    if manifest.p0_scenarios.is_empty() {
        return Err(AgentError::Prompt(
            "package manifest p0_scenarios must not be empty".into(),
        ));
    }
    Ok(())
}

fn selector_contract_safe(selector: &str) -> bool {
    !selector.trim().is_empty() && selector.len() <= 256 && !selector.chars().any(char::is_control)
}

fn comparison_id_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
}

fn html_is_self_contained(html: &str) -> bool {
    if [
        "http://",
        "https://",
        "ws://",
        "wss://",
        "ftp://",
        "file://",
        "@import",
        "<script",
        "<link ",
        "<base ",
        "<iframe",
        "<object",
        "<embed",
        "<form",
        " http-equiv=",
        " srcset=",
        " poster=",
        " action=",
        " formaction=",
        " xlink:href=",
        " ping=",
        "\"//",
        "'//",
        " onclick=",
        " onload=",
        " onerror=",
        " onchange=",
        " oninput=",
        " onsubmit=",
        "<meta http-equiv=\"refresh\"",
        "<meta http-equiv='refresh'",
    ]
    .iter()
    .any(|marker| html.contains(marker))
    {
        return false;
    }
    attributes(html, "src")
        .iter()
        .all(|value| value.starts_with("data:"))
        && attributes(html, "href").iter().all(|value| {
            value.starts_with('#')
                || value == &"gallery.html"
                // Mockups live one directory below the package gallery. This
                // exact navigation link changes no rendered bytes and cannot
                // escape to an arbitrary file or network resource.
                || value == &"../gallery.html"
                || (value.starts_with("mockups/") && value.ends_with(".html"))
        })
        && css_urls(html)
            .iter()
            .all(|value| value.starts_with("data:"))
}

fn attributes<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let mut values = Vec::new();
    for quote in ['\"', '\''] {
        let marker = format!(" {name}={quote}");
        let mut rest = html;
        while let Some(start) = rest.find(&marker) {
            let value = &rest[start + marker.len()..];
            let Some(end) = value.find(quote) else {
                values.push("");
                break;
            };
            values.push(value[..end].trim());
            rest = &value[end + quote.len_utf8()..];
        }
    }
    values
}

fn css_urls(html: &str) -> Vec<&str> {
    let mut values = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("url(") {
        let value = &rest[start + 4..];
        let Some(end) = value.find(')') else {
            values.push("");
            break;
        };
        values.push(value[..end].trim().trim_matches(['\"', '\'']));
        rest = &value[end + 1..];
    }
    values
}

fn require_regular_file(path: &Path, label: &str) -> Result<(), AgentError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| AgentError::Prompt(format!("missing mandatory package file: {label}")))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(AgentError::Prompt(format!(
            "package artifact must be a regular file: {label}"
        )));
    }
    Ok(())
}

fn regular_files(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, AgentError> {
    let mut files = std::fs::read_dir(dir)
        .map_err(|_| AgentError::Prompt(format!("missing package directory: {}", dir.display())))?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AgentError::Prompt(format!("reading package directory: {error}")))?;
    files.retain(|path| {
        path.is_file() && path.extension().and_then(|value| value.to_str()) == Some(extension)
    });
    files.sort();
    Ok(files)
}

fn read_bounded(path: &Path) -> Result<String, AgentError> {
    let bytes = std::fs::read(path)
        .map_err(|error| AgentError::Prompt(format!("reading {}: {error}", path.display())))?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(AgentError::Prompt(format!(
            "package artifact exceeds size bound: {}",
            path.display()
        )));
    }
    String::from_utf8(bytes).map_err(|_| {
        AgentError::Prompt(format!("package artifact is not UTF-8: {}", path.display()))
    })
}

fn fenced_block<'a>(text: &'a str, language: &str) -> Option<&'a str> {
    let open = format!("```{language}");
    let start = text.find(&open)? + open.len();
    let body = text.get(start..)?.strip_prefix('\n')?;
    let end = body.find("```")?;
    Some(body[..end].trim())
}

fn package_files(root: &Path) -> Result<Vec<PathBuf>, AgentError> {
    fn visit(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), AgentError> {
        for entry in std::fs::read_dir(dir)
            .map_err(|error| AgentError::Prompt(format!("reading package tree: {error}")))?
        {
            let path = entry
                .map_err(|error| AgentError::Prompt(format!("reading package tree: {error}")))?
                .path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                AgentError::Prompt(format!("reading package metadata: {error}"))
            })?;
            if metadata.file_type().is_symlink() {
                return Err(AgentError::Prompt(format!(
                    "package symlinks are forbidden: {}",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                visit(&path, files)?;
            } else if metadata.is_file() {
                files.push(path);
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, &mut files)?;
    files.sort();
    if files.len() > MAX_PACKAGE_FILES {
        return Err(AgentError::Prompt(
            "the package exceeded the file-count evidence bound".into(),
        ));
    }
    let total = files.iter().try_fold(0u64, |total, path| {
        let size = std::fs::metadata(path)
            .map_err(|error| AgentError::Prompt(format!("reading package metadata: {error}")))?
            .len();
        let total = total.saturating_add(size);
        if total > MAX_PACKAGE_BYTES {
            return Err(AgentError::Prompt(
                "the package exceeded the 10 MiB evidence bound".into(),
            ));
        }
        Ok(total)
    })?;
    let _ = total;
    Ok(files)
}

fn digest_tree(root: &Path) -> Result<String, AgentError> {
    let files = package_files(root)?;
    let mut hasher = Sha256::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| AgentError::Prompt("package path escaped root".into()))?;
        let bytes = std::fs::read(&path)
            .map_err(|error| AgentError::Prompt(format!("reading package artifact: {error}")))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(&bytes);
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Re-run every package and Git invariant without spawning an agent or
/// mutating the checkout. Expected validation failures are structured data so
/// the approval surface can explain the exact blocker.
pub async fn verify_existing(
    project_dir: &Path,
    spec: &SpecVersion,
) -> ArchitecturePackageValidation {
    match verify_existing_inner(project_dir, spec).await {
        Ok(validated) => ArchitecturePackageValidation {
            valid: true,
            package_digest: validated.package_digest,
            manifest_digest: validated.manifest_digest,
            commit_sha: spec
                .provenance
                .as_ref()
                .map(|value| value.package_commit_sha.clone())
                .unwrap_or_default(),
            tree_sha: spec
                .provenance
                .as_ref()
                .map(|value| value.package_tree_sha.clone())
                .unwrap_or_default(),
            file_count: validated.file_count,
            gallery_path: "gallery.html".into(),
            scenarios: validated.scenarios,
            findings: vec![
                ArchitectureValidationFinding {
                    code: "git_commit_tree_verified".into(),
                    message: "Pinned commit and full tree match the draft provenance.".into(),
                },
                ArchitectureValidationFinding {
                    code: "design_tree_clean".into(),
                    message: "The live design tree is byte-identical to the pinned commit.".into(),
                },
                ArchitectureValidationFinding {
                    code: "manifest_complete".into(),
                    message:
                        "Every deliverable and P0 comparison contract is declared and present."
                            .into(),
                },
                ArchitectureValidationFinding {
                    code: "static_assets_safe".into(),
                    message: "All artifacts are confined, self-contained and network-free.".into(),
                },
                ArchitectureValidationFinding {
                    code: "content_digests_verified".into(),
                    message: "Manifest and package content digests match the draft.".into(),
                },
            ],
        },
        Err(error) => invalid_validation(spec, error.to_string()),
    }
}

async fn verify_existing_inner(
    project_dir: &Path,
    spec: &SpecVersion,
) -> Result<ValidatedPackage, AgentError> {
    let provenance = spec
        .provenance
        .as_ref()
        .ok_or_else(|| AgentError::Prompt("draft has no immutable Architect provenance".into()))?;
    let design = Path::new(&spec.design_dir);
    if design.is_absolute()
        || !spec.design_dir.starts_with("design/v")
        || !spec.design_dir.ends_with('/')
        || design.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AgentError::Prompt(
            "draft design directory is outside the versioned package scope".into(),
        ));
    }

    let commit_object = format!("{}^{{commit}}", provenance.package_commit_sha);
    if git_text(project_dir, &["rev-parse", "--verify", &commit_object]).await?
        != provenance.package_commit_sha
    {
        return Err(AgentError::Prompt(
            "pinned architecture commit no longer resolves exactly".into(),
        ));
    }
    let commit_tree = format!("{}^{{tree}}", provenance.package_commit_sha);
    if git_text(project_dir, &["rev-parse", &commit_tree]).await? != provenance.package_tree_sha {
        return Err(AgentError::Prompt(
            "pinned architecture commit tree does not match the draft".into(),
        ));
    }
    if !git_success(
        project_dir,
        &[
            "merge-base",
            "--is-ancestor",
            &provenance.package_commit_sha,
            "HEAD",
        ],
    )
    .await?
    {
        return Err(AgentError::Prompt(
            "pinned architecture commit is not an ancestor of the current checkout".into(),
        ));
    }
    if !git_success(
        project_dir,
        &[
            "diff",
            "--quiet",
            &provenance.package_commit_sha,
            "HEAD",
            "--",
            &spec.design_dir,
        ],
    )
    .await?
        || !git_text(
            project_dir,
            &["status", "--porcelain=v1", "--", &spec.design_dir],
        )
        .await?
        .is_empty()
    {
        return Err(AgentError::Prompt(
            "architecture package bytes changed after the draft was created; start a new version"
                .into(),
        ));
    }

    let root = project_dir.join(spec.design_dir.trim_end_matches('/'));
    if std::fs::symlink_metadata(&root)
        .map(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir())
        .unwrap_or(true)
    {
        return Err(AgentError::Prompt(
            "architecture package root is missing or not a regular directory".into(),
        ));
    }
    let validated = validate_package(&root, &provenance.skill_digest, provenance.operating_mode)?;
    if validated.package_digest != provenance.package_digest
        || validated.manifest_digest != provenance.manifest_digest
    {
        return Err(AgentError::Prompt(
            "architecture package content digest does not match the draft".into(),
        ));
    }
    Ok(validated)
}

fn invalid_validation(spec: &SpecVersion, message: String) -> ArchitecturePackageValidation {
    let provenance = spec.provenance.as_ref();
    ArchitecturePackageValidation {
        valid: false,
        package_digest: provenance
            .map(|value| value.package_digest.clone())
            .unwrap_or_default(),
        manifest_digest: provenance
            .map(|value| value.manifest_digest.clone())
            .unwrap_or_default(),
        commit_sha: provenance
            .map(|value| value.package_commit_sha.clone())
            .unwrap_or_default(),
        tree_sha: provenance
            .map(|value| value.package_tree_sha.clone())
            .unwrap_or_default(),
        file_count: 0,
        gallery_path: "gallery.html".into(),
        scenarios: Vec::new(),
        findings: vec![ArchitectureValidationFinding {
            code: "immutable_package_invalid".into(),
            message,
        }],
    }
}

pub async fn read_artifact(
    project_dir: &Path,
    spec: &SpecVersion,
    relative_path: &str,
) -> Result<String, AgentError> {
    let verification = verify_existing(project_dir, spec).await;
    if !verification.valid {
        return Err(AgentError::Prompt(
            "architecture package is no longer valid; artifact access is blocked".into(),
        ));
    }
    let path = Path::new(relative_path);
    if relative_path.is_empty()
        || path.is_absolute()
        || !matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("md" | "html")
        )
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AgentError::Prompt(
            "only confined Markdown or HTML architecture artifacts may be read".into(),
        ));
    }
    let provenance = spec
        .provenance
        .as_ref()
        .ok_or_else(|| AgentError::Prompt("architecture artifact has no pinned commit".into()))?;
    let object = format!(
        "{}:{}{}",
        provenance.package_commit_sha, spec.design_dir, relative_path
    );
    let bytes = git_bytes(project_dir, &["show", &object]).await?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(AgentError::Prompt(
            "architecture artifact exceeds the rendering bound".into(),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| AgentError::Prompt("architecture artifact is not UTF-8".into()))
}

async fn git_success(dir: &Path, args: &[&str]) -> Result<bool, AgentError> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|error| AgentError::Prompt(format!("reading Git evidence: {error}")))?;
    Ok(output.status.success())
}

async fn git_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>, AgentError> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|error| AgentError::Prompt(format!("reading Git artifact: {error}")))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(AgentError::Prompt(
            "the requested architecture artifact is absent from the pinned commit".into(),
        ))
    }
}

async fn git_ok(dir: &Path, args: &[&str]) -> Result<(), AgentError> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|error| AgentError::Prompt(format!("running isolated Git step: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(AgentError::Prompt(format!(
            "isolated Git step failed: {}",
            truncate(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
                1024
            )
        )))
    }
}

async fn git_text(dir: &Path, args: &[&str]) -> Result<String, AgentError> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|error| AgentError::Prompt(format!("reading Git evidence: {error}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(AgentError::Prompt(
            "the project is not a healthy Git checkout for architecture generation".into(),
        ))
    }
}

fn path_text(path: &Path) -> Result<String, AgentError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| AgentError::Prompt("worktree path is not valid UTF-8".into()))
}

fn truncate(mut value: String, limit: usize) -> String {
    if value.len() > limit {
        let mut boundary = limit;
        while !value.is_char_boundary(boundary) {
            boundary -= 1;
        }
        value.truncate(boundary);
        value.push_str("\n… truncated");
    }
    value
}

#[cfg(test)]
mod html_safety_tests {
    use super::{
        bind_manifest_provenance, html_is_self_contained, validate_manifest_header,
        ArchitectureOperatingMode, ManifestDeliverable, PackageManifest,
    };

    #[test]
    fn static_gallery_links_and_data_assets_are_allowed() {
        assert!(html_is_self_contained(
            r#"<html><a href="mockups/home.html">Home</a><img src="data:image/png;base64,AA=="><style>.x{background:url('data:image/png;base64,AA==')}</style></html>"#,
        ));
    }

    #[test]
    fn scripts_and_every_network_shaped_dependency_are_rejected() {
        for html in [
            r#"<script>fetch('/secret')</script>"#,
            r#"<img src="//example.com/image.png">"#,
            r#"<style>@import "theme.css";</style>"#,
            r#"<div style="background:url(asset.png)"></div>"#,
            r#"<button onclick="fetch('/x')">X</button>"#,
        ] {
            assert!(
                !html_is_self_contained(html),
                "accepted unsafe HTML: {html}"
            );
        }
    }

    #[test]
    fn manifest_header_failures_are_specific_and_bounded() {
        let digest = "a".repeat(64);
        let valid = || PackageManifest {
            schema_version: 2,
            skill_digest: digest.clone(),
            operating_mode: "greenfield".into(),
            deliverables: vec![ManifestDeliverable {
                path: "package-manifest.md".into(),
                kind: "manifest".into(),
            }],
            p0_scenarios: vec![],
        };
        let error = validate_manifest_header(
            &valid(),
            &digest,
            ArchitectureOperatingMode::Greenfield,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "the prompt failed: package manifest p0_scenarios must not be empty");

        let mut wrong_digest = valid();
        wrong_digest.skill_digest = "b".repeat(64);
        let error = validate_manifest_header(
            &wrong_digest,
            &digest,
            ArchitectureOperatingMode::Greenfield,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("skill_digest"));
        assert!(!error.contains(&digest));
    }

    #[test]
    fn server_binds_manifest_provenance_instead_of_trusting_model_echoes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("package-manifest.md"),
            "```latoile-package\n{\"schema_version\":99,\"skill_digest\":\"wrong\",\"operating_mode\":\"wrong\",\"deliverables\":[],\"p0_scenarios\":[]}\n```\n",
        )
        .unwrap();
        let digest = "a".repeat(64);

        bind_manifest_provenance(
            root.path(),
            &digest,
            ArchitectureOperatingMode::Greenfield,
        )
        .unwrap();

        let bound = std::fs::read_to_string(root.path().join("package-manifest.md")).unwrap();
        assert!(bound.contains(&digest));
        assert!(bound.contains("\"operating_mode\": \"greenfield\""));
        assert!(bound.contains("\"schema_version\": 2"));
        assert!(!bound.contains("\"wrong\""));
    }
}

#[cfg(test)]
#[path = "architecture_package/qa_regression_issue_008.rs"]
mod qa_regression_issue_008;

#[cfg(test)]
#[path = "architecture_package/qa_regression_issue_011.rs"]
mod qa_regression_issue_011;

#[cfg(test)]
#[path = "architecture_package/qa_regression_issue_012.rs"]
mod qa_regression_issue_012;
