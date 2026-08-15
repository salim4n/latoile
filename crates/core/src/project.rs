//! `Project` — the central entity. Everything else hangs off it.

use crate::error::DomainError;
use crate::ids::ProjectId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectStatus {
    Draft,
    Specced,
    Building,
    Live,
}

impl ProjectStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectStatus::Draft => "draft",
            ProjectStatus::Specced => "specced",
            ProjectStatus::Building => "building",
            ProjectStatus::Live => "live",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub slug: String,
    /// `owner/name` on GitHub.
    pub github_repo: String,
    pub default_branch: String,
    /// The single integration branch all runs commit to (ADR-004).
    pub work_branch: String,
    /// Checkout path on the host running LaToile.
    pub local_path: String,
    pub status: ProjectStatus,
    /// e.g. `pnpm dev --port $PORT` — how the preview starts the app.
    pub dev_command: String,
    pub deleted: bool,
}

impl Project {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ProjectId,
        name: impl Into<String>,
        slug: impl Into<String>,
        github_repo: impl Into<String>,
        work_branch: impl Into<String>,
        local_path: impl Into<String>,
        dev_command: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        let slug = slug.into();
        let github_repo = github_repo.into();
        if name.trim().is_empty() || slug.trim().is_empty() {
            return Err(DomainError::Invariant("a project needs a name and a slug"));
        }
        if !github_repo.contains('/') {
            return Err(DomainError::Invariant("github_repo must look like owner/name"));
        }
        Ok(Self {
            id,
            name,
            slug,
            github_repo,
            default_branch: "main".into(),
            work_branch: work_branch.into(),
            local_path: local_path.into(),
            status: ProjectStatus::Draft,
            dev_command: dev_command.into(),
            deleted: false,
        })
    }

    /// The first spec got approved.
    pub fn mark_specced(&mut self) {
        if self.status == ProjectStatus::Draft {
            self.status = ProjectStatus::Specced;
        }
    }

    /// Real work started on the board.
    pub fn mark_building(&mut self) {
        if matches!(self.status, ProjectStatus::Draft | ProjectStatus::Specced) {
            self.status = ProjectStatus::Building;
        }
    }

    /// The app is serving users of its own.
    pub fn mark_live(&mut self) {
        if self.status == ProjectStatus::Building {
            self.status = ProjectStatus::Live;
        }
    }

    /// Soft delete — the only entity that gets one.
    pub fn soft_delete(&mut self) {
        self.deleted = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        Project::new(
            ProjectId::new("p1").unwrap(),
            "Mon App",
            "mon-app",
            "salim4n/mon-app",
            "work",
            "/srv/latoile/projects/mon-app",
            "pnpm dev --port $PORT",
        )
        .unwrap()
    }

    #[test]
    fn repo_must_look_like_owner_slash_name() {
        assert!(Project::new(
            ProjectId::new("p1").unwrap(),
            "X",
            "x",
            "no-slash",
            "work",
            "/tmp/x",
            "pnpm dev",
        )
        .is_err());
    }

    #[test]
    fn status_progression_never_regresses() {
        let mut p = project();
        p.mark_live(); // no-op: not building yet
        assert_eq!(p.status, ProjectStatus::Draft);
        p.mark_specced();
        p.mark_building();
        p.mark_specced(); // no-op: already past
        assert_eq!(p.status, ProjectStatus::Building);
        p.mark_live();
        assert_eq!(p.status, ProjectStatus::Live);
    }
}
