//! Git-backed project checkout provisioning. The adapter owns filesystem
//! layout, authentication and branch discovery; the application receives
//! only canonical facts safe to persist.

use crate::{GitHub, GitHubError};
use base64::Engine as _;
use latoile_core::ports::{
    PortResult, ProvisionWorkspaceInput, ProvisionedWorkspace, PublishWorkBranchInput,
    PublishedWorkBranch, SecretStore, WorkBranchPublisher, WorkspaceProvisioner,
};
use std::path::Path;
use tokio::process::Command;

const FALLBACK_DEV_COMMAND: &str =
    "printf 'LaToile: no dev command detected; configure dev_command for this project\\n' >&2; exit 64";

impl<S: SecretStore> WorkspaceProvisioner for GitHub<S> {
    async fn provision(&self, input: &ProvisionWorkspaceInput) -> PortResult<ProvisionedWorkspace> {
        provision(self, input).await.map_err(Into::into)
    }
}

impl<S: SecretStore> WorkBranchPublisher for GitHub<S> {
    async fn verify_and_push(
        &self,
        input: &PublishWorkBranchInput,
    ) -> PortResult<PublishedWorkBranch> {
        publish_work_branch(self, input).await.map_err(Into::into)
    }
}

async fn publish_work_branch<S: SecretStore>(
    github: &GitHub<S>,
    input: &PublishWorkBranchInput,
) -> Result<PublishedWorkBranch, GitHubError> {
    let (owner, name) = repo_parts(&input.repo)?;
    validate_component(owner, "repository owner")?;
    validate_component(name, "repository name")?;
    validate_branch(&input.work_branch)?;

    let root = tokio::fs::canonicalize(&github.config.workspace_root)
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))?;
    let checkout = tokio::fs::canonicalize(&input.checkout)
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))?;
    if !checkout.starts_with(&root) || !checkout.join(".git").is_dir() {
        return Err(GitHubError::Workspace(
            "the delivery checkout is outside the configured workspace or is not a Git checkout"
                .into(),
        ));
    }

    let token = github.token().await?;
    let origin = git_output(&checkout, &token, &["remote", "get-url", "origin"]).await?;
    if !remote_matches(&origin, &input.repo) {
        return Err(GitHubError::Workspace(
            "the delivery checkout origin does not match the project repository".into(),
        ));
    }
    let branch = git_output(&checkout, &token, &["branch", "--show-current"]).await?;
    if branch != input.work_branch {
        return Err(GitHubError::Workspace(format!(
            "delivery requires branch {}, but the checkout is on {branch}",
            input.work_branch
        )));
    }
    let status = git_output(
        &checkout,
        &token,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .await?;
    if !status.is_empty() {
        return Err(GitHubError::Workspace(
            "delivery requires a clean worktree with every selected change committed".into(),
        ));
    }

    let local_sha = git_output(&checkout, &token, &["rev-parse", "HEAD"]).await?;
    if local_sha.len() < 40 || !local_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GitHubError::Workspace(
            "Git returned an invalid local commit SHA".into(),
        ));
    }
    if input.approved_shas.is_empty() {
        return Err(GitHubError::Workspace(
            "delivery needs at least one approved executor SHA".into(),
        ));
    }
    for approved in &input.approved_shas {
        if approved.len() < 40 || !approved.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(GitHubError::Workspace(
                "an approved executor SHA is invalid".into(),
            ));
        }
        let ancestor = Command::new("git")
            .args(["merge-base", "--is-ancestor", approved, &local_sha])
            .current_dir(&checkout)
            .env("GIT_TERMINAL_PROMPT", "0")
            .status()
            .await
            .map_err(|error| GitHubError::Workspace(error.to_string()))?;
        if !ancestor.success() {
            return Err(GitHubError::Workspace(format!(
                "approved executor SHA {approved} is not contained in the delivery HEAD"
            )));
        }
    }
    let refspec = format!("HEAD:refs/heads/{}", input.work_branch);
    git(
        &checkout,
        &token,
        &["push", "--porcelain", "origin", &refspec],
    )
    .await?;
    let remote_ref = format!("refs/heads/{}", input.work_branch);
    let remote = git_output(
        &checkout,
        &token,
        &["ls-remote", "--heads", "origin", &remote_ref],
    )
    .await?;
    let remote_sha = remote
        .split_whitespace()
        .next()
        .filter(|sha| !sha.is_empty())
        .ok_or_else(|| GitHubError::Workspace("the pushed branch is absent on origin".into()))?
        .to_string();
    if remote_sha != local_sha {
        return Err(GitHubError::Workspace(format!(
            "remote SHA verification failed: local {local_sha}, remote {remote_sha}"
        )));
    }

    Ok(PublishedWorkBranch {
        work_branch: input.work_branch.clone(),
        local_sha,
        remote_sha,
    })
}

async fn provision<S: SecretStore>(
    github: &GitHub<S>,
    input: &ProvisionWorkspaceInput,
) -> Result<ProvisionedWorkspace, GitHubError> {
    validate_component(&input.slug, "slug")?;
    let (owner, name) = repo_parts(&input.repo)?;
    validate_component(owner, "repository owner")?;
    validate_component(name, "repository name")?;
    validate_branch(&input.work_branch)?;

    tokio::fs::create_dir_all(&github.config.workspace_root)
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))?;
    let root = tokio::fs::canonicalize(&github.config.workspace_root)
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))?;
    let owner_dir = root.join(owner);
    tokio::fs::create_dir_all(&owner_dir)
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))?;
    let owner_dir = tokio::fs::canonicalize(owner_dir)
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))?;
    if !owner_dir.starts_with(&root) {
        return Err(GitHubError::Workspace(
            "the repository path escapes the configured workspace".into(),
        ));
    }

    let destination = owner_dir.join(&input.slug);
    let token = github.token().await?;
    let remote = format!(
        "{}/{}/{}.git",
        github.config.git_remote_base.trim_end_matches('/'),
        owner,
        name
    );

    if destination.exists() {
        if !destination.join(".git").is_dir() {
            return Err(GitHubError::Workspace(format!(
                "{} exists but is not a Git checkout",
                destination.display()
            )));
        }
        let origin = git_output(&destination, &token, &["remote", "get-url", "origin"]).await?;
        if !remote_matches(&origin, &input.repo) {
            return Err(GitHubError::Workspace(format!(
                "{} is a checkout of a different repository",
                destination.display()
            )));
        }
        git(&destination, &token, &["fetch", "origin", "--prune"]).await?;
    } else {
        clone_atomically(&owner_dir, &destination, &remote, &token).await?;
    }

    let canonical = tokio::fs::canonicalize(&destination)
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))?;
    if !canonical.starts_with(&root) {
        return Err(GitHubError::Workspace(
            "the checkout resolved outside the configured workspace".into(),
        ));
    }

    let default_branch = default_branch(&canonical, &token).await?;
    checkout_work_branch(&canonical, &token, &input.work_branch, &default_branch).await?;
    let dev_command = match input.dev_command.as_deref().map(str::trim) {
        Some(command) if !command.is_empty() => command.to_string(),
        _ => detect_dev_command(&canonical)
            .await
            .unwrap_or_else(|| FALLBACK_DEV_COMMAND.into()),
    };

    Ok(ProvisionedWorkspace {
        default_branch,
        work_branch: input.work_branch.clone(),
        local_path: canonical.to_string_lossy().into_owned(),
        dev_command,
    })
}

fn remote_matches(remote: &str, repo: &str) -> bool {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let repo = repo.trim_end_matches(".git");
    remote.ends_with(&format!("/{repo}")) || remote.ends_with(&format!(":{repo}"))
}

fn repo_parts(repo: &str) -> Result<(&str, &str), GitHubError> {
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty() => {
            Ok((owner, name))
        }
        _ => Err(GitHubError::Workspace(
            "repository must look like owner/name".into(),
        )),
    }
}

fn validate_component(value: &str, label: &str) -> Result<(), GitHubError> {
    let valid = !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(GitHubError::Workspace(format!("invalid {label}")))
    }
}

fn validate_branch(value: &str) -> Result<(), GitHubError> {
    if value.is_empty()
        || value.starts_with('-')
        || value.contains("..")
        || value.contains([' ', '~', '^', ':', '?', '*', '[', '\\'])
    {
        return Err(GitHubError::Workspace("invalid work branch".into()));
    }
    Ok(())
}

async fn clone_atomically(
    parent: &Path,
    destination: &Path,
    remote: &str,
    token: &str,
) -> Result<(), GitHubError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".latoile-clone-{}-{stamp}", std::process::id()));
    let temporary_text = temporary.to_string_lossy().into_owned();
    let result = git(
        parent,
        token,
        &["clone", "--origin", "origin", remote, &temporary_text],
    )
    .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_dir_all(&temporary).await;
        return Err(error);
    }
    tokio::fs::rename(&temporary, destination)
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))
}

async fn default_branch(checkout: &Path, token: &str) -> Result<String, GitHubError> {
    let remote_head = git_output(
        checkout,
        token,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .await?;
    remote_head
        .trim()
        .strip_prefix("origin/")
        .filter(|branch| !branch.is_empty())
        .map(str::to_string)
        .ok_or_else(|| GitHubError::Workspace("origin has no default branch".into()))
}

async fn checkout_work_branch(
    checkout: &Path,
    token: &str,
    work: &str,
    default: &str,
) -> Result<(), GitHubError> {
    if git_ref_exists(checkout, token, &format!("refs/heads/{work}")).await? {
        git(checkout, token, &["checkout", work]).await
    } else if git_ref_exists(checkout, token, &format!("refs/remotes/origin/{work}")).await? {
        git(
            checkout,
            token,
            &["checkout", "-B", work, &format!("origin/{work}")],
        )
        .await
    } else {
        git(
            checkout,
            token,
            &["checkout", "-B", work, &format!("origin/{default}")],
        )
        .await
    }
}

async fn git_ref_exists(
    checkout: &Path,
    token: &str,
    reference: &str,
) -> Result<bool, GitHubError> {
    let status = git_command(checkout, token)
        .args(["show-ref", "--verify", "--quiet", reference])
        .status()
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))?;
    Ok(status.success())
}

async fn git(checkout: &Path, token: &str, args: &[&str]) -> Result<(), GitHubError> {
    git_output(checkout, token, args).await.map(|_| ())
}

async fn git_output(checkout: &Path, token: &str, args: &[&str]) -> Result<String, GitHubError> {
    let output = git_command(checkout, token)
        .args(args)
        .output()
        .await
        .map_err(|e| GitHubError::Workspace(e.to_string()))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).replace(token, "[redacted]");
    Err(GitHubError::Workspace(format!(
        "git exited with {}: {}",
        output.status,
        stderr.trim()
    )))
}

fn git_command(checkout: &Path, token: &str) -> Command {
    let credentials =
        base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
    let mut command = Command::new("git");
    command
        .current_dir(checkout)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Basic {credentials}"),
        );
    command
}

async fn detect_dev_command(checkout: &Path) -> Option<String> {
    let package = checkout.join("package.json");
    if let Ok(bytes) = tokio::fs::read(&package).await {
        let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        if json
            .pointer("/scripts/dev")
            .and_then(|v| v.as_str())
            .is_some()
        {
            let (runner, separator) = if checkout.join("pnpm-lock.yaml").exists() {
                ("pnpm", "--")
            } else if checkout.join("yarn.lock").exists() {
                ("yarn", "")
            } else if checkout.join("bun.lock").exists() || checkout.join("bun.lockb").exists() {
                ("bun run", "--")
            } else {
                ("npm run", "--")
            };
            return Some(format!("{runner} dev {separator} --port $PORT").replace("  ", " "));
        }
    }
    checkout
        .join("Cargo.toml")
        .exists()
        .then(|| "cargo run".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use latoile_core::ports::{PortError, SecretStore};
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[derive(Clone)]
    struct Secrets(HashMap<String, String>);

    impl SecretStore for Secrets {
        async fn get(&self, name: &str) -> Result<Option<String>, PortError> {
            Ok(self.0.get(name).cloned())
        }
        async fn put(&self, _: &str, _: &str) -> Result<(), PortError> {
            Ok(())
        }
    }

    fn commit_repo(root: &Path) -> PathBuf {
        let source = root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::process::Command::new("git")
            .args(["init", "-b", "trunk"])
            .current_dir(&source)
            .status()
            .unwrap();
        std::fs::write(source.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();
        std::fs::write(source.join("pnpm-lock.yaml"), "lockfileVersion: 9").unwrap();
        for args in [
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "initial",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&source)
                .status()
                .unwrap()
                .success());
        }
        let remote = root.join("remotes/salim4n/mon-app.git");
        std::fs::create_dir_all(remote.parent().unwrap()).unwrap();
        assert!(std::process::Command::new("git")
            .args([
                "clone",
                "--bare",
                source.to_str().unwrap(),
                remote.to_str().unwrap()
            ])
            .status()
            .unwrap()
            .success());
        remote
    }

    #[tokio::test]
    async fn provisions_and_reuses_a_real_checkout() {
        let temp = tempfile::tempdir().unwrap();
        commit_repo(temp.path());
        let config = crate::GitHubConfig {
            workspace_root: temp.path().join("workspace"),
            git_remote_base: format!("file://{}", temp.path().join("remotes").display()),
            ..crate::GitHubConfig::default()
        };
        let github = GitHub::new(
            config,
            Secrets(HashMap::from([(
                crate::DEFAULT_TOKEN_NAME.into(),
                "never-log-me".into(),
            )])),
            GitHub::<Secrets>::default_http(),
        );
        let input = ProvisionWorkspaceInput {
            repo: "salim4n/mon-app".into(),
            slug: "mon-app".into(),
            work_branch: "work".into(),
            dev_command: None,
        };

        let first = github.provision(&input).await.unwrap();
        let second = github.provision(&input).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(first.default_branch, "trunk");
        assert_eq!(first.dev_command, "pnpm dev -- --port $PORT");
        assert!(Path::new(&first.local_path).join(".git").is_dir());
        assert_eq!(
            git_output(
                Path::new(&first.local_path),
                "x",
                &["branch", "--show-current"]
            )
            .await
            .unwrap(),
            "work"
        );
    }

    #[tokio::test]
    async fn rejects_workspace_escape_before_running_git() {
        let temp = tempfile::tempdir().unwrap();
        let github = GitHub::new(
            crate::GitHubConfig {
                workspace_root: temp.path().join("workspace"),
                ..crate::GitHubConfig::default()
            },
            Secrets(HashMap::new()),
            GitHub::<Secrets>::default_http(),
        );
        let error = github
            .provision(&ProvisionWorkspaceInput {
                repo: "salim4n/mon-app".into(),
                slug: "../outside".into(),
                work_branch: "work".into(),
                dev_command: None,
            })
            .await
            .unwrap_err();
        assert!(error.0.contains("invalid slug"));
    }

    #[tokio::test]
    async fn publishes_a_clean_work_branch_and_verifies_the_remote_sha() {
        let temp = tempfile::tempdir().unwrap();
        let remote = commit_repo(temp.path());
        let github = GitHub::new(
            crate::GitHubConfig {
                workspace_root: temp.path().join("workspace"),
                git_remote_base: format!("file://{}", temp.path().join("remotes").display()),
                ..crate::GitHubConfig::default()
            },
            Secrets(HashMap::from([(
                crate::DEFAULT_TOKEN_NAME.into(),
                "never-log-me".into(),
            )])),
            GitHub::<Secrets>::default_http(),
        );
        let provisioned = github
            .provision(&ProvisionWorkspaceInput {
                repo: "salim4n/mon-app".into(),
                slug: "mon-app".into(),
                work_branch: "work".into(),
                dev_command: None,
            })
            .await
            .unwrap();
        let checkout = Path::new(&provisioned.local_path);
        std::fs::write(checkout.join("feature.txt"), "ready").unwrap();
        for args in [
            vec!["add", "feature.txt"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "feat: ready",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(checkout)
                .status()
                .unwrap()
                .success());
        }

        let approved_sha = git_output(checkout, "x", &["rev-parse", "HEAD"])
            .await
            .unwrap();
        let published = github
            .verify_and_push(&PublishWorkBranchInput {
                repo: "salim4n/mon-app".into(),
                checkout: provisioned.local_path,
                work_branch: "work".into(),
                approved_shas: vec![approved_sha],
            })
            .await
            .unwrap();
        assert_eq!(published.local_sha, published.remote_sha);
        let remote_sha = std::process::Command::new("git")
            .args([
                "--git-dir",
                remote.to_str().unwrap(),
                "rev-parse",
                "refs/heads/work",
            ])
            .output()
            .unwrap();
        assert!(remote_sha.status.success());
        assert_eq!(
            String::from_utf8_lossy(&remote_sha.stdout).trim(),
            published.local_sha
        );
    }

    #[tokio::test]
    async fn refuses_a_dirty_or_wrong_branch_before_push() {
        let temp = tempfile::tempdir().unwrap();
        commit_repo(temp.path());
        let github = GitHub::new(
            crate::GitHubConfig {
                workspace_root: temp.path().join("workspace"),
                git_remote_base: format!("file://{}", temp.path().join("remotes").display()),
                ..crate::GitHubConfig::default()
            },
            Secrets(HashMap::from([(
                crate::DEFAULT_TOKEN_NAME.into(),
                "never-log-me".into(),
            )])),
            GitHub::<Secrets>::default_http(),
        );
        let provisioned = github
            .provision(&ProvisionWorkspaceInput {
                repo: "salim4n/mon-app".into(),
                slug: "mon-app".into(),
                work_branch: "work".into(),
                dev_command: None,
            })
            .await
            .unwrap();
        std::fs::write(
            Path::new(&provisioned.local_path).join("dirty.txt"),
            "dirty",
        )
        .unwrap();
        let error = github
            .verify_and_push(&PublishWorkBranchInput {
                repo: "salim4n/mon-app".into(),
                checkout: provisioned.local_path,
                work_branch: "work".into(),
                approved_shas: vec![],
            })
            .await
            .unwrap_err();
        assert!(error.0.contains("clean worktree"), "{error}");
        assert!(!error.0.contains("never-log-me"));
    }
}
