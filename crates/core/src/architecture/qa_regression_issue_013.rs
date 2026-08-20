use super::{ArchitectureSession, ArchitectureSessionId, ProjectId};

#[test]
fn original_owner_brief_is_pinned_before_provider_start() {
    let mut session = ArchitectureSession::new(
        ArchitectureSessionId::new("architecture-brief").unwrap(),
        ProjectId::new("project-brief").unwrap(),
    );
    session
        .record_brief("Exactly one mobile P0 scenario.")
        .unwrap();
    assert_eq!(session.brief, "Exactly one mobile P0 scenario.");

    assert!(session.record_brief("Expanded scope").is_err());
    session.attach_agent("acp:brief").unwrap();
    assert!(session.record_brief("Late mutation").is_err());
}

#[test]
fn empty_owner_brief_cannot_become_package_authority() {
    let mut session = ArchitectureSession::new(
        ArchitectureSessionId::new("architecture-empty-brief").unwrap(),
        ProjectId::new("project-empty-brief").unwrap(),
    );
    assert!(session.record_brief("  ").is_err());
}
