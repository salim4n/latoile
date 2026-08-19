//! Wire DTOs. Core types deliberately derive no serde, so every route maps
//! to an explicit shape here — the API contract is visible in one place and
//! a domain refactor can't silently change the wire.

use latoile_app::store::{
    InboxApprovalRow, ProjectListRow, ProjectMessageRow, ProjectTaskRow, RoleRow,
};
use latoile_core::{
    Approval, ArchitectureQuestion, ArchitectureSession, Delivery, Message, Preview, Project, Run,
    SpecVersion, Task, VisualBaseline, VisualComparison,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
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
            last_activity_at: None,
        }
    }
}

impl From<&ProjectListRow> for ProjectDto {
    fn from(row: &ProjectListRow) -> Self {
        let mut dto = Self::from(&row.project);
        dto.last_activity_at = Some(row.last_activity_at.clone());
        dto
    }
}

#[derive(Serialize)]
pub struct ArchitectureQuestionDto {
    pub id: String,
    pub sequence: u32,
    pub prompt: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}

#[derive(Serialize)]
pub struct ArchitectureSessionDto {
    pub id: String,
    pub project_id: String,
    pub status: &'static str,
    pub phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operating_mode: Option<&'static str>,
    pub requested_locale: String,
    pub package_status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<ArchitecturePackageDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub questions: Vec<ArchitectureQuestionDto>,
}

#[derive(Serialize)]
pub struct ArchitecturePackageDto {
    pub design_dir: String,
    pub base_sha: String,
    pub head_sha: String,
    pub tree_sha: String,
    pub package_digest: String,
    pub manifest_digest: String,
    pub changed_files: Vec<String>,
    pub diff_stat: String,
}

impl ArchitectureSessionDto {
    pub fn new(session: &ArchitectureSession, questions: &[ArchitectureQuestion]) -> Self {
        Self {
            id: session.id.as_str().to_string(),
            project_id: session.project_id.as_str().to_string(),
            status: session.status.as_str(),
            phase: session.phase.as_str(),
            skill_name: session.skill_name.clone(),
            skill_digest: session.skill_digest.clone(),
            operating_mode: session.operating_mode.map(|mode| mode.as_str()),
            requested_locale: session.requested_locale.clone(),
            package_status: session.package_status.as_str(),
            package: session
                .package
                .as_ref()
                .map(|package| ArchitecturePackageDto {
                    design_dir: package.design_dir.clone(),
                    base_sha: package.base_sha.clone(),
                    head_sha: package.head_sha.clone(),
                    tree_sha: package.tree_sha.clone(),
                    package_digest: package.package_digest.clone(),
                    manifest_digest: package.manifest_digest.clone(),
                    changed_files: package.changed_files.clone(),
                    diff_stat: package.diff_stat.clone(),
                }),
            failure_reason: session.failure_reason.clone(),
            questions: questions
                .iter()
                .map(|question| ArchitectureQuestionDto {
                    id: question.id.as_str().to_string(),
                    sequence: question.sequence,
                    prompt: question.prompt.clone(),
                    status: question.status.as_str(),
                    answer: question.answer.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
pub struct DeliveryDto {
    pub status: &'static str,
    pub work_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request_url: Option<String>,
}

impl DeliveryDto {
    pub fn not_started(project: &Project) -> Self {
        Self {
            status: "not_started",
            work_branch: project.work_branch.clone(),
            local_sha: None,
            remote_sha: None,
            pull_request_url: None,
        }
    }
}

impl From<&Delivery> for DeliveryDto {
    fn from(delivery: &Delivery) -> Self {
        Self {
            status: delivery.status.as_str(),
            work_branch: delivery.work_branch.clone(),
            local_sha: Some(delivery.local_sha.clone()),
            remote_sha: Some(delivery.remote_sha.clone()),
            pull_request_url: delivery.pull_request_url.clone(),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operating_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_commit_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_tree_sha: Option<String>,
}

impl From<&SpecVersion> for SpecDto {
    fn from(s: &SpecVersion) -> Self {
        let provenance = s.provenance.as_ref();
        Self {
            id: s.id.as_str().to_string(),
            project_id: s.project_id.as_str().to_string(),
            version: s.version,
            status: s.status.as_str(),
            design_dir: s.design_dir.clone(),
            architecture_session_id: provenance
                .map(|value| value.architecture_session_id.as_str().to_string()),
            skill_name: provenance.map(|value| value.skill_name.clone()),
            skill_digest: provenance.map(|value| value.skill_digest.clone()),
            operating_mode: provenance.map(|value| value.operating_mode.as_str()),
            package_digest: provenance.map(|value| value.package_digest.clone()),
            manifest_digest: provenance.map(|value| value.manifest_digest.clone()),
            package_commit_sha: provenance.map(|value| value.package_commit_sha.clone()),
            package_tree_sha: provenance.map(|value| value.package_tree_sha.clone()),
        }
    }
}

#[derive(Serialize)]
pub struct ArchitectureValidationFindingDto {
    pub code: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct ArchitectureVisualScenarioDto {
    pub comparison_id: String,
    pub screen: String,
    pub state: String,
    pub locale: String,
    pub theme: String,
    pub route: String,
    pub fixture: String,
    pub readiness_selector: String,
    pub stable_selectors: Vec<String>,
    pub allowed_masks: Vec<String>,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub device_scale_factor_milli: u32,
    pub mockup: String,
}

#[derive(Serialize)]
pub struct ArchitecturePackageValidationDto {
    pub valid: bool,
    pub package_digest: String,
    pub manifest_digest: String,
    pub commit_sha: String,
    pub tree_sha: String,
    pub file_count: u32,
    pub gallery_path: String,
    pub scenarios: Vec<ArchitectureVisualScenarioDto>,
    pub findings: Vec<ArchitectureValidationFindingDto>,
}

impl From<&latoile_core::ArchitecturePackageValidation> for ArchitecturePackageValidationDto {
    fn from(value: &latoile_core::ArchitecturePackageValidation) -> Self {
        Self {
            valid: value.valid,
            package_digest: value.package_digest.clone(),
            manifest_digest: value.manifest_digest.clone(),
            commit_sha: value.commit_sha.clone(),
            tree_sha: value.tree_sha.clone(),
            file_count: value.file_count,
            gallery_path: value.gallery_path.clone(),
            scenarios: value
                .scenarios
                .iter()
                .map(|scenario| ArchitectureVisualScenarioDto {
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
                    viewport_width: scenario.viewport_width,
                    viewport_height: scenario.viewport_height,
                    device_scale_factor_milli: scenario.device_scale_factor_milli,
                    mockup: scenario.mockup.clone(),
                })
                .collect(),
            findings: value
                .findings
                .iter()
                .map(|finding| ArchitectureValidationFindingDto {
                    code: finding.code.clone(),
                    message: finding.message.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
pub struct VisualBaselineDto {
    pub spec_version_id: String,
    pub comparison_id: String,
    pub manifest_digest: String,
    pub package_commit_sha: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub png_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accessibility_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<String>,
}

impl From<&VisualBaseline> for VisualBaselineDto {
    fn from(value: &VisualBaseline) -> Self {
        Self {
            spec_version_id: value.spec_version_id.as_str().to_string(),
            comparison_id: value.comparison_id.clone(),
            manifest_digest: value.manifest_digest.clone(),
            package_commit_sha: value.package_commit_sha.clone(),
            status: value.status.as_str(),
            png_digest: value.png_digest.clone(),
            geometry_digest: value.geometry_digest.clone(),
            accessibility_digest: value.accessibility_digest.clone(),
            environment_digest: value.environment_digest.clone(),
            browser_version: value.browser_version.clone(),
            font_fingerprint: value.font_fingerprint.clone(),
            failure_code: value.failure_code.clone(),
            failure_message: value.failure_message.clone(),
            recovery_action: value.recovery_action.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct VisualComparisonDto {
    pub id: String,
    pub spec_version_id: String,
    pub project_id: String,
    pub run_id: String,
    pub comparison_id: String,
    pub manifest_digest: String,
    pub package_commit_sha: String,
    pub baseline_png_digest: String,
    pub status: &'static str,
    pub changed_pixels: u64,
    pub total_pixels: u64,
    pub pixel_ratio_micros: u32,
    pub max_geometry_delta_milli: u32,
    pub accessibility_changes: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_png_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pixel_diff_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heatmap_png_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry_diff_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accessibility_diff_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<String>,
}

impl From<&VisualComparison> for VisualComparisonDto {
    fn from(value: &VisualComparison) -> Self {
        Self {
            id: value.id.as_str().to_string(),
            spec_version_id: value.spec_version_id.as_str().to_string(),
            project_id: value.project_id.as_str().to_string(),
            run_id: value.run_id.as_str().to_string(),
            comparison_id: value.comparison_id.clone(),
            manifest_digest: value.manifest_digest.clone(),
            package_commit_sha: value.package_commit_sha.clone(),
            baseline_png_digest: value.baseline_png_digest.clone(),
            status: value.status.as_str(),
            changed_pixels: value.changed_pixels,
            total_pixels: value.total_pixels,
            pixel_ratio_micros: value.pixel_ratio_micros,
            max_geometry_delta_milli: value.max_geometry_delta_milli,
            accessibility_changes: value.accessibility_changes,
            render_png_digest: value.render_png_digest.clone(),
            pixel_diff_digest: value.pixel_diff_digest.clone(),
            heatmap_png_digest: value.heatmap_png_digest.clone(),
            geometry_diff_digest: value.geometry_diff_digest.clone(),
            accessibility_diff_digest: value.accessibility_diff_digest.clone(),
            environment_digest: value.environment_digest.clone(),
            browser_version: value.browser_version.clone(),
            font_fingerprint: value.font_fingerprint.clone(),
            failure_code: value.failure_code.clone(),
            failure_message: value.failure_message.clone(),
            recovery_action: value.recovery_action.clone(),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_review_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_decision_comment: Option<String>,
    pub next_action: &'static str,
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
            latest_run_id: None,
            latest_review_status: None,
            latest_decision_comment: None,
            next_action: next_action(t.status.as_str(), None),
        }
    }
}

impl From<&ProjectTaskRow> for TaskDto {
    fn from(row: &ProjectTaskRow) -> Self {
        let mut dto = Self::from(&row.task);
        dto.latest_run_id.clone_from(&row.latest_run_id);
        dto.latest_review_status
            .clone_from(&row.latest_review_status);
        dto.latest_decision_comment
            .clone_from(&row.latest_decision_comment);
        dto.next_action = next_action(
            row.task.status.as_str(),
            row.latest_review_status.as_deref(),
        );
        dto
    }
}

fn next_action(task_status: &str, latest_review: Option<&str>) -> &'static str {
    match (task_status, latest_review) {
        ("done", _) => "completed",
        ("review", Some("pending")) => "awaiting_owner_decision",
        ("review", _) => "reviewer_working",
        ("in_progress", Some("rejected")) => "corrective_run_in_progress",
        ("in_progress", _) => "agent_working",
        ("changes_requested", _) => "changes_requested",
        ("ready", Some("rejected")) => "correction_ready",
        _ => "ready_to_start",
    }
}

#[derive(Serialize)]
pub struct RunDto {
    pub id: String,
    pub task_id: String,
    pub role_id: String,
    pub status: &'static str,
    pub summary: Option<String>,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub artifacts: Option<serde_json::Value>,
}

impl From<&Run> for RunDto {
    fn from(r: &Run) -> Self {
        Self {
            id: r.id.as_str().to_string(),
            task_id: r.task_id.as_str().to_string(),
            role_id: r.role_id.as_str().to_string(),
            status: r.status.as_str(),
            summary: r.summary.clone(),
            base_sha: r.base_sha.clone(),
            head_sha: r.head_sha.clone(),
            artifacts: r
                .artifacts
                .as_deref()
                .and_then(|value| serde_json::from_str(value).ok()),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corrective_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
}

impl From<&Approval> for ApprovalDto {
    fn from(a: &Approval) -> Self {
        Self {
            id: a.id.as_str().to_string(),
            run_id: a.run_id.as_str().to_string(),
            kind: a.kind.as_str(),
            status: a.status.as_str(),
            payload: a.payload.clone(),
            decision_comment: a.decision_comment.clone(),
            corrective_run_id: a
                .corrective_run_id
                .as_ref()
                .map(|run| run.as_str().to_string()),
            project_id: None,
            project_name: None,
            task_title: None,
            role_id: None,
            created_at: None,
            decided_at: None,
        }
    }
}

impl From<&InboxApprovalRow> for ApprovalDto {
    fn from(row: &InboxApprovalRow) -> Self {
        Self {
            id: row.approval.id.as_str().to_string(),
            run_id: row.approval.run_id.as_str().to_string(),
            kind: row.approval.kind.as_str(),
            status: row.approval.status.as_str(),
            payload: row.approval.payload.clone(),
            decision_comment: row.approval.decision_comment.clone(),
            corrective_run_id: row
                .approval
                .corrective_run_id
                .as_ref()
                .map(|run| run.as_str().to_string()),
            project_id: Some(row.project_id.clone()),
            project_name: Some(row.project_name.clone()),
            task_title: Some(row.task_title.clone()),
            role_id: Some(row.role_id.clone()),
            created_at: Some(row.created_at.clone()),
            decided_at: row.decided_at.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct MessageDto {
    pub id: String,
    pub author: &'static str,
    pub content: String,
    pub actions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

impl From<&Message> for MessageDto {
    fn from(m: &Message) -> Self {
        Self {
            id: m.id.as_str().to_string(),
            author: m.author.as_str(),
            content: m.content.clone(),
            actions: m.actions.clone(),
            created_at: None,
        }
    }
}

impl From<&ProjectMessageRow> for MessageDto {
    fn from(row: &ProjectMessageRow) -> Self {
        let mut dto = Self::from(&row.message);
        dto.created_at = Some(row.created_at.clone());
        dto
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
    pub logs: Vec<String>,
}

impl PreviewDto {
    pub fn of(p: &Preview, alive: bool, logs: Vec<String>) -> Self {
        Self {
            id: p.id.as_str().to_string(),
            project_id: p.project_id.as_str().to_string(),
            port: p.port,
            status: p.status.as_str(),
            branch: p.branch.clone(),
            alive,
            logs,
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
