//! Isolated Architect package generation. The agent writes in a detached
//! temporary Git worktree, never in the project's live checkout. LaToile
//! validates the complete static package and the exact path-level diff,
//! commits it itself, then integrates only that verified commit by fast
//! forward.

use crate::config::{AgentCommand, AgentTimeouts};
use crate::error::AgentError;
use crate::preamble::ArchitectSkillBundle;
use crate::transport::{Connection, Connector, PermissionContext};
use crate::updates::RunOutcome;
use latoile_core::ports::{ArchitecturePackageReply, ArchitecturePackageRequest};
use latoile_core::{ArchitectureOperatingMode, ArchitecturePackageEvidence, ArchitectureSessionId};
use serde::Deserialize;
use sha2::{Digest, Sha256};
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageManifest {
    schema_version: u32,
    skill_digest: String,
    operating_mode: String,
    p0_scenarios: Vec<P0Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct P0Scenario {
    id: String,
    screen: String,
    mockup: String,
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
    let turn = tokio::time::timeout(timeouts.prompt, conn.prompt(&prompt))
        .await
        .map_err(|_| {
            AgentError::Timeout(format!(
                "architecture package (session {}, cwd {})",
                session.as_str(),
                worktree.display()
            ))
        })??;
    if turn.outcome != RunOutcome::Finished {
        return Err(AgentError::Prompt(
            "the Architect did not finish the package turn".into(),
        ));
    }

    let changed_files = changed_files(worktree).await?;
    validate_changed_paths(&changed_files, &request.design_dir)?;
    let package_digest = validate_package(&package_root, request, bundle)?;

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
            package_digest,
            changed_files: committed_paths,
            diff_stat: truncate(diff_stat, 32 * 1024),
        },
        summary: truncate(turn.text.trim().to_string(), 16 * 1024),
    })
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
    format!(
        "{}\n\n---\n\nPACKAGE-ONLY AUTHORITY\nOperating mode: {}\nPinned skill SHA-256: {}\nWrite ONLY under `{}`. Do not execute commands. Do not modify source, configuration, scripts, dependencies or files outside that directory. Produce specifications, Mermaid diagrams and self-contained static HTML only.\n\nDURABLE OWNER DECISIONS\n{}\n\nMANDATORY PACKAGE CONTRACT\nCreate every file below:\n- package-manifest.md\n- architecture-spec.md\n- domain-model.md\n- data-model.md\n- api-contract.md\n- architecture-blueprint.md\n- component-specification.md\n- stack-decisions.md\n- architecture-contract.md\n- guardian-checklist.md\n- user-flows.md\n- screen-inventory.md\n- design-tokens.md\n- gallery.html\n- adrs/ADR-001-*.md (at least one ADR)\n- mockups/<scenario>.html (one self-contained page for every P0 scenario)\n\n`package-manifest.md` MUST contain exactly one fenced `latoile-package` JSON object with schema_version 1, the pinned skill_digest, operating_mode, and non-empty p0_scenarios entries containing id, screen and mockup (path relative to the package). Every P0 id must appear in screen-inventory.md. Gallery must link every P0 mockup. Compute SHA-256 of the exact `design-tokens.md` bytes and include `data-latoile-token-digest=\"<digest>\"` on the root element of gallery.html and every mockup. No external assets or network URLs. Finish with a concise summary; LaToile validates and commits the package.",
        bundle.render(),
        request.operating_mode.as_str(),
        request.skill_digest,
        request.design_dir,
        decisions,
    )
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

fn validate_package(
    root: &Path,
    request: &ArchitecturePackageRequest,
    bundle: &ArchitectSkillBundle,
) -> Result<String, AgentError> {
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

    let manifest_text = read_bounded(&root.join("package-manifest.md"))?;
    let manifest_raw = fenced_block(&manifest_text, "latoile-package").ok_or_else(|| {
        AgentError::Prompt("package-manifest.md is missing the latoile-package contract".into())
    })?;
    let manifest: PackageManifest = serde_json::from_str(manifest_raw).map_err(|error| {
        AgentError::Prompt(format!("invalid latoile-package manifest: {error}"))
    })?;
    if manifest.schema_version != 1
        || manifest.skill_digest != bundle.digest
        || manifest.operating_mode != request.operating_mode.as_str()
        || manifest.p0_scenarios.is_empty()
    {
        return Err(AgentError::Prompt(
            "the package manifest does not match the pinned skill, mode and P0 contract".into(),
        ));
    }

    let inventory = read_bounded(&root.join("screen-inventory.md"))?;
    let gallery = read_bounded(&root.join("gallery.html"))?;
    for scenario in &manifest.p0_scenarios {
        if scenario.id.trim().is_empty()
            || scenario.screen.trim().is_empty()
            || !scenario.mockup.starts_with("mockups/")
            || Path::new(&scenario.mockup)
                .extension()
                .and_then(|value| value.to_str())
                != Some("html")
        {
            return Err(AgentError::Prompt(
                "every P0 scenario needs an id, screen and mockups/*.html path".into(),
            ));
        }
        require_regular_file(&root.join(&scenario.mockup), &scenario.mockup)?;
        if !inventory.contains(&scenario.id) || !gallery.contains(&scenario.mockup) {
            return Err(AgentError::Prompt(format!(
                "P0 scenario {} is not traceable through inventory and gallery",
                scenario.id
            )));
        }
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
    for html in std::iter::once(root.join("gallery.html")).chain(mockups.into_iter()) {
        let text = read_bounded(&html)?;
        let lowered = text.to_ascii_lowercase();
        if !text.contains(&format!("data-latoile-token-digest=\"{token_digest}\""))
            || lowered.contains("<script src=")
            || lowered.contains("<link rel=")
            || lowered.contains("http://")
            || lowered.contains("https://")
        {
            return Err(AgentError::Prompt(format!(
                "HTML artifact {} is external or does not pin the shared design tokens",
                html.display()
            )));
        }
    }

    digest_tree(root)
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

fn digest_tree(root: &Path) -> Result<String, AgentError> {
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
    let mut total = 0u64;
    let mut hasher = Sha256::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| AgentError::Prompt("package path escaped root".into()))?;
        let bytes = std::fs::read(&path)
            .map_err(|error| AgentError::Prompt(format!("reading package artifact: {error}")))?;
        total = total.saturating_add(bytes.len() as u64);
        if total > MAX_PACKAGE_BYTES {
            return Err(AgentError::Prompt(
                "the package exceeded the 10 MiB evidence bound".into(),
            ));
        }
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
