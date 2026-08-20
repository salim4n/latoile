//! The `GitHubClient` port over the REST API.
//!
//! The token is resolved through the `SecretStore` port (the vault) at each
//! call — never from the environment, never cached in a struct field, so a
//! rotated token takes effect on the next call and no copy of it sits in
//! this crate's memory between calls.
//!
//! Two operations, exactly the port's surface: list the owner's repos (the
//! project picker) and open a pull request. Nothing here merges — nothing
//! merges without the owner's explicit approval, and a PR is how the work
//! branch asks for one.

use crate::error::GitHubError;
use latoile_core::ports::{GitHubClient, PortResult, RepoInfo, SecretStore};
use serde::Deserialize;
use std::path::PathBuf;

/// The default secret name in the vault.
pub const DEFAULT_TOKEN_NAME: &str = "github_token";

#[derive(Debug, Clone)]
pub struct GitHubConfig {
    /// `https://api.github.com` in production; tests point it at a mock.
    pub api_base: String,
    pub token_name: String,
    /// Root below which project checkouts are provisioned.
    pub workspace_root: PathBuf,
    /// Git smart-HTTP base. Tests use a local file URL; production uses
    /// GitHub without putting credentials in the URL.
    pub git_remote_base: String,
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            api_base: "https://api.github.com".into(),
            token_name: DEFAULT_TOKEN_NAME.into(),
            workspace_root: PathBuf::from("workspace"),
            git_remote_base: "https://github.com".into(),
        }
    }
}

#[derive(Clone)]
pub struct GitHub<S> {
    pub(crate) config: GitHubConfig,
    pub(crate) secrets: S,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct RepoJson {
    full_name: String,
    description: Option<String>,
    private: bool,
}

/// The 422 body: `{"message": "...", "errors": [...]}`. The message is the
/// part worth surfacing.
#[derive(Deserialize)]
struct ValidationBody {
    message: String,
}

/// The 201 answer to a PR creation: the URL is what the UI links to.
#[derive(Deserialize)]
struct PullJson {
    html_url: String,
}

impl<S: SecretStore> GitHub<S> {
    pub fn new(config: GitHubConfig, secrets: S, http: reqwest::Client) -> Self {
        Self {
            config,
            secrets,
            http,
        }
    }

    /// A client with GitHub's one hard requirement (a User-Agent) set.
    pub fn default_http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent(concat!("latoile/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("a reqwest client with default settings builds")
    }

    pub(crate) async fn token(&self) -> Result<String, GitHubError> {
        self.secrets
            .get(&self.config.token_name)
            .await
            .map_err(|e| GitHubError::Decode(e.to_string()))?
            .ok_or_else(|| GitHubError::TokenMissing(self.config.token_name.clone()))
    }

    /// Map a response's status to an error, or hand it back for decoding.
    async fn checked(
        response: reqwest::Response,
        what: &str,
    ) -> Result<reqwest::Response, GitHubError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(match status.as_u16() {
            401 | 403 => GitHubError::Auth,
            404 => GitHubError::NotFound(what.to_string()),
            422 => GitHubError::Validation(
                serde_json::from_str::<ValidationBody>(&body)
                    .map(|b| b.message)
                    .unwrap_or(body),
            ),
            other => GitHubError::Decode(format!("status {other}: {body}")),
        })
    }
}

impl<S: SecretStore> GitHubClient for GitHub<S> {
    async fn list_repos(&self) -> PortResult<Vec<RepoInfo>> {
        let token = self.token().await?;
        let response = self
            .http
            .get(format!("{}/user/repos", self.config.api_base))
            .query(&[("per_page", "100"), ("sort", "updated")])
            .bearer_auth(token)
            .send()
            .await
            .map_err(GitHubError::from)?;
        let repos = Self::checked(response, "your repositories")
            .await?
            .json::<Vec<RepoJson>>()
            .await
            .map_err(|e| GitHubError::Decode(e.to_string()))?;
        Ok(repos
            .into_iter()
            .map(|r| RepoInfo {
                full_name: r.full_name,
                description: r.description,
                private: r.private,
            })
            .collect())
    }

    /// Open the PR from `head` to `base`; returns its URL. If GitHub answers
    /// 422 because the PR already exists, that message is surfaced as-is —
    /// the use case decides whether to fetch the existing one.
    async fn open_pull_request(&self, repo: &str, head: &str, base: &str) -> PortResult<String> {
        let token = self.token().await?;
        let response = self
            .http
            .post(format!("{}/repos/{repo}/pulls", self.config.api_base))
            .bearer_auth(token)
            .json(&serde_json::json!({
                "title": head,
                "head": head,
                "base": base,
                "body": "Opened by LaToile — the work branch asks for review.",
            }))
            .send()
            .await
            .map_err(GitHubError::from)?;
        let url = Self::checked(response, repo)
            .await?
            .json::<PullJson>()
            .await
            .map_err(|e| GitHubError::Decode(e.to_string()))?;
        Ok(url.html_url)
    }

    async fn find_open_pull_request(
        &self,
        repo: &str,
        head: &str,
        base: &str,
    ) -> PortResult<Option<String>> {
        let owner = repo
            .split_once('/')
            .map(|(owner, _)| owner)
            .filter(|owner| !owner.is_empty())
            .ok_or_else(|| {
                GitHubError::Validation("repository must look like owner/name".into())
            })?;
        let token = self.token().await?;
        let response = self
            .http
            .get(format!("{}/repos/{repo}/pulls", self.config.api_base))
            .bearer_auth(token)
            .query(&[
                ("state", "open".to_string()),
                ("head", format!("{owner}:{head}")),
                ("base", base.to_string()),
                ("per_page", "1".to_string()),
            ])
            .send()
            .await
            .map_err(GitHubError::from)?;
        let pulls = Self::checked(response, repo)
            .await?
            .json::<Vec<PullJson>>()
            .await
            .map_err(|e| GitHubError::Decode(e.to_string()))?;
        Ok(pulls.into_iter().next().map(|pull| pull.html_url))
    }
}

#[cfg(test)]
mod tests;
