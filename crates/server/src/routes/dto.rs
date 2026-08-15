//! Wire DTOs. Core types deliberately derive no serde, so every route maps
//! to an explicit shape here — the API contract is visible in one place and
//! a domain refactor can't silently change the wire.

use latoile_app::store::RoleRow;
use latoile_core::{
    Approval, Message, Preview, Project, Run, SpecVersion, Task,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub github_repo: String,
    pub default_branch: String,
    pub work_branch: String,
    pub local_path: String,
    pub status: &'static str,
    pub dev_command: String,
}

impl From<&Project> for ProjectDto {
    fn from(p: &Project) -> Self {
        Self {
            id: p.id.as_str().to_string(),
            name: p.name.clone(),
            slug: p.slug.clone(),
            github_repo: p.github_repo.clone(),
            default_branch: p.default_branch.clone(),
            work_branch: p.work_branch.clone(),
            local_path: p.local_path.clone(),
            status: p.status.as_str(),
            dev_command: p.dev_command.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct SpecDto {
    pub id: String,
    pub project_id: String,
    pub version: u32,
    pub status: &'static str,
    pub design_dir: String,
}

impl From<&SpecVersion> for SpecDto {
    fn from(s: &SpecVersion) -> Self {
        Self {
            id: s.id.as_str().to_string(),
            project_id: s.project_id.as_str().to_string(),
            version: s.version,
            status: s.status.as_str(),
            design_dir: s.design_dir.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct TaskDto {
    pub id: String,
    pub project_id: String,
    pub spec_version_id: Option<String>,
    pub role_id: String,
    pub title: String,
    pub description: String,
    pub status: &'static str,
    pub position: u32,
}

impl From<&Task> for TaskDto {
    fn from(t: &Task) -> Self {
        Self {
            id: t.id.as_str().to_string(),
            project_id: t.project_id.as_str().to_string(),
            spec_version_id: t.spec_version_id.as_ref().map(|s| s.as_str().to_string()),
            role_id: t.role_id.as_str().to_string(),
            title: t.title.clone(),
            description: t.description.clone(),
            status: t.status.as_str(),
            position: t.position,
        }
    }
}

#[derive(Serialize)]
pub struct RunDto {
    pub id: String,
    pub task_id: String,
    pub role_id: String,
    pub status: &'static str,
    pub summary: Option<String>,
}

impl From<&Run> for RunDto {
    fn from(r: &Run) -> Self {
        Self {
            id: r.id.as_str().to_string(),
            task_id: r.task_id.as_str().to_string(),
            role_id: r.role_id.as_str().to_string(),
            status: r.status.as_str(),
            summary: r.summary.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct ApprovalDto {
    pub id: String,
    pub run_id: String,
    pub kind: &'static str,
    pub status: &'static str,
    pub payload: String,
}

impl From<&Approval> for ApprovalDto {
    fn from(a: &Approval) -> Self {
        Self {
            id: a.id.as_str().to_string(),
            run_id: a.run_id.as_str().to_string(),
            kind: a.kind.as_str(),
            status: a.status.as_str(),
            payload: a.payload.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct MessageDto {
    pub id: String,
    pub author: &'static str,
    pub content: String,
    pub actions: Option<String>,
}

impl From<&Message> for MessageDto {
    fn from(m: &Message) -> Self {
        Self {
            id: m.id.as_str().to_string(),
            author: m.author.as_str(),
            content: m.content.clone(),
            actions: m.actions.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct PreviewDto {
    pub id: String,
    pub project_id: String,
    pub port: u16,
    pub status: &'static str,
    pub branch: String,
    pub alive: bool,
}

impl PreviewDto {
    pub fn of(p: &Preview, alive: bool) -> Self {
        Self {
            id: p.id.as_str().to_string(),
            project_id: p.project_id.as_str().to_string(),
            port: p.port,
            status: p.status.as_str(),
            branch: p.branch.clone(),
            alive,
        }
    }
}

#[derive(Serialize)]
pub struct RoleDto {
    pub id: String,
    pub label: String,
    pub skill_path: Option<String>,
    pub cli: String,
    pub system_prompt_path: Option<String>,
}

impl From<&RoleRow> for RoleDto {
    fn from(r: &RoleRow) -> Self {
        Self {
            id: r.id.clone(),
            label: r.label.clone(),
            skill_path: r.skill_path.clone(),
            cli: r.cli.clone(),
            system_prompt_path: r.system_prompt_path.clone(),
        }
    }
}
