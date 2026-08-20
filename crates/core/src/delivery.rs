//! Audited delivery of the project's one work branch. A delivery is created
//! only after Git proved that the pushed remote SHA equals the clean local
//! checkout; opening a PR is a second, explicit state.

use crate::error::DomainError;
use crate::ids::ProjectId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pushed,
    PullRequestOpen,
}

impl DeliveryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pushed => "pushed",
            Self::PullRequestOpen => "pull_request_open",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub project_id: ProjectId,
    pub work_branch: String,
    pub local_sha: String,
    pub remote_sha: String,
    pub status: DeliveryStatus,
    pub pull_request_url: Option<String>,
}

impl Delivery {
    pub fn pushed(
        project_id: ProjectId,
        work_branch: impl Into<String>,
        local_sha: impl Into<String>,
        remote_sha: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let work_branch = work_branch.into();
        let local_sha = local_sha.into();
        let remote_sha = remote_sha.into();
        if work_branch.trim().is_empty() || local_sha.trim().is_empty() {
            return Err(DomainError::Invariant(
                "delivery needs a branch and a verified SHA",
            ));
        }
        if local_sha != remote_sha {
            return Err(DomainError::Invariant(
                "the pushed remote SHA must equal the local SHA",
            ));
        }
        Ok(Self {
            project_id,
            work_branch,
            local_sha,
            remote_sha,
            status: DeliveryStatus::Pushed,
            pull_request_url: None,
        })
    }

    pub fn attach_pull_request(&mut self, url: impl Into<String>) -> Result<(), DomainError> {
        let url = url.into();
        if url.trim().is_empty() {
            return Err(DomainError::Invariant("delivery needs a Pull Request URL"));
        }
        self.pull_request_url = Some(url);
        self.status = DeliveryStatus::PullRequestOpen;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_must_match_local_before_a_pr_can_be_attached() {
        let project = ProjectId::new("p1").unwrap();
        assert!(Delivery::pushed(project.clone(), "work", "a", "b").is_err());
        let mut delivery = Delivery::pushed(project, "work", "abc", "abc").unwrap();
        delivery
            .attach_pull_request("https://github.com/acme/app/pull/1")
            .unwrap();
        assert_eq!(delivery.status, DeliveryStatus::PullRequestOpen);
    }
}
