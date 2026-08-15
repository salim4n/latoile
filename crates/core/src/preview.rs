//! `Preview` — the supervised dev server of a project. One active preview per
//! project (enforced with the application layer's partial unique index). A
//! preview that no longer serves the work branch reports `stale` or `error`;
//! it never silently serves the wrong thing (invariant §3.2.6).

use crate::error::{DomainError, TransitionError};
use crate::ids::{PreviewId, ProjectId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    Starting,
    Ready,
    Stale,
    Error,
    Stopped,
}

impl PreviewStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PreviewStatus::Starting => "starting",
            PreviewStatus::Ready => "ready",
            PreviewStatus::Stale => "stale",
            PreviewStatus::Error => "error",
            PreviewStatus::Stopped => "stopped",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, PreviewStatus::Starting | PreviewStatus::Ready | PreviewStatus::Stale)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub id: PreviewId,
    pub project_id: ProjectId,
    pub port: u16,
    pub status: PreviewStatus,
    pub branch: String,
    pub pid: Option<u32>,
}

impl Preview {
    pub fn new(id: PreviewId, project_id: ProjectId, port: u16, branch: impl Into<String>) -> Self {
        Self {
            id,
            project_id,
            port,
            status: PreviewStatus::Starting,
            branch: branch.into(),
            pid: None,
        }
    }

    fn go(&mut self, to: PreviewStatus) -> Result<(), DomainError> {
        let allowed = matches!(
            (self.status, to),
            (PreviewStatus::Starting, PreviewStatus::Ready)
                | (PreviewStatus::Starting, PreviewStatus::Error)
                | (PreviewStatus::Ready, PreviewStatus::Stale)
                | (PreviewStatus::Stale, PreviewStatus::Ready)
                | (PreviewStatus::Ready, PreviewStatus::Error)
                | (PreviewStatus::Stale, PreviewStatus::Error)
        );
        if !allowed {
            return Err(
                TransitionError::new("preview", self.status.as_str(), to.as_str()).into(),
            );
        }
        self.status = to;
        Ok(())
    }

    pub fn mark_ready(&mut self, pid: u32) -> Result<(), DomainError> {
        self.go(PreviewStatus::Ready)?;
        self.pid = Some(pid);
        Ok(())
    }

    pub fn mark_stale(&mut self) -> Result<(), DomainError> {
        self.go(PreviewStatus::Stale)
    }

    /// The branch moved back or a rebuild finished: fresh again.
    /// Only a stale preview can refresh — a starting one uses `mark_ready`.
    pub fn refresh(&mut self) -> Result<(), DomainError> {
        if self.status != PreviewStatus::Stale {
            return Err(
                TransitionError::new("preview", self.status.as_str(), "ready").into(),
            );
        }
        self.status = PreviewStatus::Ready;
        Ok(())
    }

    pub fn fail(&mut self) -> Result<(), DomainError> {
        self.go(PreviewStatus::Error)
    }

    pub fn stop(&mut self) -> Result<(), DomainError> {
        if !self.status.is_active() && self.status != PreviewStatus::Error {
            return Err(
                TransitionError::new("preview", self.status.as_str(), "stopped").into(),
            );
        }
        self.status = PreviewStatus::Stopped;
        self.pid = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview() -> Preview {
        Preview::new(
            PreviewId::new("pr1").unwrap(),
            ProjectId::new("p1").unwrap(),
            4100,
            "work",
        )
    }

    #[test]
    fn lifecycle_ready_stale_refresh_stop() {
        let mut p = preview();
        p.mark_ready(4242).unwrap();
        p.mark_stale().unwrap();
        p.refresh().unwrap();
        assert_eq!(p.status, PreviewStatus::Ready);
        p.stop().unwrap();
        assert!(p.pid.is_none());
        assert!(p.mark_ready(1).is_err()); // Stopped is terminal
    }

    #[test]
    fn refused_transitions() {
        let mut p = preview();
        assert!(p.mark_stale().is_err()); // Starting → Stale
        assert!(p.refresh().is_err()); // Starting → Ready via refresh
        p.fail().unwrap();
        assert!(p.mark_ready(1).is_err()); // Error → Ready requires a restart
        p.stop().unwrap(); // Error → Stopped is allowed
    }
}
