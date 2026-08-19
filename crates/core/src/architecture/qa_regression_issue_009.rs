use super::{
    ArchitectureOperatingMode, ArchitecturePackageStatus, ArchitectureSession,
    ArchitectureSessionId, ProjectId, ARCHITECT_SKILL_NAME,
};

fn generating_session() -> ArchitectureSession {
    let mut session = ArchitectureSession::new(
        ArchitectureSessionId::new("architecture-issue-009").unwrap(),
        ProjectId::new("project-issue-009").unwrap(),
    );
    session.attach_agent("acp:issue-009").unwrap();
    session
        .record_skill(
            ARCHITECT_SKILL_NAME,
            "a".repeat(64),
            ArchitectureOperatingMode::Greenfield,
        )
        .unwrap();
    session.ready_to_draft().unwrap();
    session.begin_package().unwrap();
    session
}

#[test]
fn failed_generation_cannot_remain_marked_as_in_progress() {
    let mut session = generating_session();

    session.fail("validator rejected the package").unwrap();

    assert_eq!(session.package_status, ArchitecturePackageStatus::NotStarted);
    assert!(!session.needs_live_process());
}

#[test]
fn cancelled_generation_cannot_remain_marked_as_in_progress() {
    let mut session = generating_session();

    session.cancel().unwrap();

    assert_eq!(session.package_status, ArchitecturePackageStatus::NotStarted);
    assert!(!session.needs_live_process());
}
