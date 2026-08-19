//! Turn a completed Socratic discovery into one verified, reproducible draft.
//! The agent adapter owns the isolated worktree and static-package checks;
//! this use case pins the durable Q/A transcript, provenance and draft.

use super::UseCaseError;
use crate::store::Store;
use latoile_core::ids::{MessageId, ProjectId, SpecVersionId};
use latoile_core::ports::{
    AgentChannel, ArchitectureDecision, ArchitecturePackageRequest, ArchitectureSessionStore,
    ConversationStore, EventLog,
};
use latoile_core::{
    ArchitectureQuestionStatus, ArchitectureSession, Author, EventKind, Message, NewEvent,
    SpecProvenance, SpecVersion,
};

pub(crate) async fn produce_architecture_package<A: AgentChannel>(
    store: &Store,
    agents: &A,
    project: &ProjectId,
    session: &mut ArchitectureSession,
) -> Result<Message, UseCaseError> {
    let questions = store.questions_for_session(&session.id).await?;
    let decisions = questions
        .iter()
        .map(|question| {
            if question.status != ArchitectureQuestionStatus::Answered {
                return Err(latoile_core::DomainError::Invariant(
                    "architecture generation requires every discovery question to be answered",
                ));
            }
            Ok(ArchitectureDecision {
                sequence: question.sequence,
                prompt: question.prompt.clone(),
                answer: question
                    .answer
                    .clone()
                    .ok_or(latoile_core::DomainError::Invariant(
                        "an answered architecture question needs its durable answer",
                    ))?,
            })
        })
        .collect::<Result<Vec<_>, latoile_core::DomainError>>()?;
    if decisions.is_empty() {
        return Err(latoile_core::DomainError::Invariant(
            "the Architect must challenge at least one owner decision before drafting",
        )
        .into());
    }

    let version = store.specs_for_project(project).await?.len() as u32 + 1;
    let session_slug = session
        .id
        .as_str()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    let design_dir = format!("design/v{version:04}-{session_slug}/");
    let skill_name = session
        .skill_name
        .clone()
        .ok_or(latoile_core::DomainError::Invariant(
            "architecture session has no pinned skill name",
        ))?;
    let skill_digest = session
        .skill_digest
        .clone()
        .ok_or(latoile_core::DomainError::Invariant(
            "architecture session has no pinned skill digest",
        ))?;
    let operating_mode = session
        .operating_mode
        .ok_or(latoile_core::DomainError::Invariant(
            "architecture session has no pinned operating mode",
        ))?;

    session.begin_package()?;
    ArchitectureSessionStore::save(store, session).await?;
    let generated = match agents
        .generate_architecture_package(
            project,
            &session.id,
            &ArchitecturePackageRequest {
                design_dir: design_dir.clone(),
                skill_digest: skill_digest.clone(),
                operating_mode,
                requested_locale: session.requested_locale.clone(),
                decisions,
            },
        )
        .await
    {
        Ok(generated) => generated,
        Err(error) => {
            session.fail(format!("architecture package rejected: {error}"))?;
            ArchitectureSessionStore::save(store, session).await?;
            return Err(error.into());
        }
    };
    session.finish_package(generated.evidence.clone())?;

    let mut spec = SpecVersion::new(
        SpecVersionId::new(ulid::Ulid::new().to_string())?,
        project.clone(),
        version,
        design_dir.clone(),
        None,
    )?;
    spec.attach_provenance(SpecProvenance {
        architecture_session_id: session.id.clone(),
        skill_name: skill_name.clone(),
        skill_digest: skill_digest.clone(),
        operating_mode,
        package_digest: generated.evidence.package_digest.clone(),
        manifest_digest: generated.evidence.manifest_digest.clone(),
        package_commit_sha: generated.evidence.head_sha.clone(),
        package_tree_sha: generated.evidence.tree_sha.clone(),
    })?;
    store.save_architecture_draft(session, &spec).await?;

    let conversation = store
        .for_project(project)
        .await?
        .ok_or(UseCaseError::NotFound("conversation"))?;
    let actions = serde_json::json!([{
        "type": "architecture_package",
        "title": format!("Paquet architecture v{version} prêt"),
        "sub": format!(
            "{} · {} fichiers · commit {}",
            design_dir,
            generated.evidence.changed_files.len(),
            &generated.evidence.head_sha[..generated.evidence.head_sha.len().min(12)]
        ),
        "spec_version_id": spec.id.as_str(),
        "skill_name": skill_name,
        "skill_digest": skill_digest,
        "operating_mode": operating_mode.as_str(),
        "package_digest": generated.evidence.package_digest,
        "manifest_digest": generated.evidence.manifest_digest,
        "package_commit_sha": generated.evidence.head_sha,
        "package_tree_sha": generated.evidence.tree_sha,
    }]);
    let message = Message::new(
        MessageId::new(ulid::Ulid::new().to_string())?,
        conversation.id,
        Author::Manager,
        "L'Architecte a produit un paquet confiné et vérifié. La spec attend maintenant votre validation.",
        Some(actions.to_string()),
    )?;
    ConversationStore::append(store, &message).await?;
    EventLog::append(
        store,
        &NewEvent {
            project_id: project.clone(),
            kind: EventKind::MessagePosted,
            payload: format!("{{\"message_id\":\"{}\"}}", message.id),
        },
    )
    .await?;
    Ok(message)
}
