use super::{ArchitectureSession, ArchitectureSessionId, ProjectId};

#[test]
fn owner_locale_is_pinned_before_the_architect_starts() {
    let mut session = ArchitectureSession::new(
        ArchitectureSessionId::new("architecture-locale").unwrap(),
        ProjectId::new("project-locale").unwrap(),
    );
    session.set_requested_locale("fr-FR").unwrap();
    assert_eq!(session.requested_locale, "fr-FR");

    session.attach_agent("acp:locale").unwrap();
    assert!(session.set_requested_locale("en-US").is_err());
}

#[test]
fn unsupported_package_locale_is_rejected() {
    let mut session = ArchitectureSession::new(
        ArchitectureSessionId::new("architecture-invalid-locale").unwrap(),
        ProjectId::new("project-invalid-locale").unwrap(),
    );
    assert!(session.set_requested_locale("es-ES").is_err());
}
