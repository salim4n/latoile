//! Architecture discovery persistence. Session and question transitions are
//! performed by core before upsert; SQLite mirrors the single-active-session
//! and single-open-question invariants with partial unique indexes.

use super::{unknown_variant, Store, StoreError};
use latoile_core::ports::{ArchitectureSessionStore, PortResult};
use latoile_core::{
    ArchitecturePhase, ArchitectureQuestion, ArchitectureQuestionId, ArchitectureQuestionStatus,
    ArchitectureSession, ArchitectureSessionId, ArchitectureStatus, ProjectId,
};
use sqlx::Row;

fn status(raw: &str) -> Result<ArchitectureStatus, StoreError> {
    Ok(match raw {
        "discovering" => ArchitectureStatus::Discovering,
        "awaiting_answer" => ArchitectureStatus::AwaitingAnswer,
        "ready_to_draft" => ArchitectureStatus::ReadyToDraft,
        "failed" => ArchitectureStatus::Failed,
        "cancelled" => ArchitectureStatus::Cancelled,
        other => return Err(unknown_variant("architecture session status", other)),
    })
}

fn phase(raw: &str) -> Result<ArchitecturePhase, StoreError> {
    Ok(match raw {
        "domain_discovery" => ArchitecturePhase::DomainDiscovery,
        "requirements" => ArchitecturePhase::Requirements,
        "ux_discovery" => ArchitecturePhase::UxDiscovery,
        "ready_to_draft" => ArchitecturePhase::ReadyToDraft,
        other => return Err(unknown_variant("architecture phase", other)),
    })
}

fn question_status(raw: &str) -> Result<ArchitectureQuestionStatus, StoreError> {
    Ok(match raw {
        "open" => ArchitectureQuestionStatus::Open,
        "answered" => ArchitectureQuestionStatus::Answered,
        other => return Err(unknown_variant("architecture question status", other)),
    })
}

fn map_session(row: &sqlx::sqlite::SqliteRow) -> Result<ArchitectureSession, StoreError> {
    Ok(ArchitectureSession {
        id: ArchitectureSessionId::new(row.try_get::<String, _>("id")?)?,
        project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)?,
        status: status(&row.try_get::<String, _>("status")?)?,
        phase: phase(&row.try_get::<String, _>("phase")?)?,
        acp_session_id: row.try_get("acp_session_id")?,
        failure_reason: row.try_get("failure_reason")?,
    })
}

fn map_question(row: &sqlx::sqlite::SqliteRow) -> Result<ArchitectureQuestion, StoreError> {
    Ok(ArchitectureQuestion {
        id: ArchitectureQuestionId::new(row.try_get::<String, _>("id")?)?,
        session_id: ArchitectureSessionId::new(row.try_get::<String, _>("session_id")?)?,
        sequence: u32::try_from(row.try_get::<i64, _>("sequence")?).map_err(|_| {
            StoreError::CorruptRow("negative architecture question sequence".into())
        })?,
        prompt: row.try_get("prompt")?,
        status: question_status(&row.try_get::<String, _>("status")?)?,
        answer: row.try_get("answer")?,
    })
}

const SESSION_COLUMNS: &str = "id, project_id, status, phase, acp_session_id, failure_reason";
const QUESTION_COLUMNS: &str = "id, session_id, sequence, prompt, status, answer";

impl Store {
    pub async fn active_architecture_sessions(&self) -> PortResult<Vec<ArchitectureSession>> {
        let rows = sqlx::query(&format!(
            "SELECT {SESSION_COLUMNS} FROM architecture_session
             WHERE status IN ('discovering', 'awaiting_answer', 'ready_to_draft')
             ORDER BY created_at ASC"
        ))
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(map_session)
            .collect::<Result<Vec<_>, StoreError>>()
            .map_err(Into::into)
    }
}

impl ArchitectureSessionStore for Store {
    async fn get(&self, id: &ArchitectureSessionId) -> PortResult<Option<ArchitectureSession>> {
        let row = sqlx::query(&format!(
            "SELECT {SESSION_COLUMNS} FROM architecture_session WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.as_ref()
            .map(map_session)
            .transpose()
            .map_err(Into::into)
    }

    async fn latest_for_project(
        &self,
        project: &ProjectId,
    ) -> PortResult<Option<ArchitectureSession>> {
        let row = sqlx::query(&format!(
            "SELECT {SESSION_COLUMNS} FROM architecture_session
             WHERE project_id = ? ORDER BY created_at DESC, id DESC LIMIT 1"
        ))
        .bind(project.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.as_ref()
            .map(map_session)
            .transpose()
            .map_err(Into::into)
    }

    async fn active_for_project(
        &self,
        project: &ProjectId,
    ) -> PortResult<Option<ArchitectureSession>> {
        let row = sqlx::query(&format!(
            "SELECT {SESSION_COLUMNS} FROM architecture_session
             WHERE project_id = ? AND status IN ('discovering', 'awaiting_answer', 'ready_to_draft')
             LIMIT 1"
        ))
        .bind(project.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.as_ref()
            .map(map_session)
            .transpose()
            .map_err(Into::into)
    }

    async fn save(&self, session: &ArchitectureSession) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO architecture_session
               (id, project_id, status, phase, acp_session_id, failure_reason)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               status = excluded.status,
               phase = excluded.phase,
               acp_session_id = excluded.acp_session_id,
               failure_reason = excluded.failure_reason,
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(session.id.as_str())
        .bind(session.project_id.as_str())
        .bind(session.status.as_str())
        .bind(session.phase.as_str())
        .bind(&session.acp_session_id)
        .bind(&session.failure_reason)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn question(
        &self,
        id: &ArchitectureQuestionId,
    ) -> PortResult<Option<ArchitectureQuestion>> {
        let row = sqlx::query(&format!(
            "SELECT {QUESTION_COLUMNS} FROM architecture_question WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.as_ref()
            .map(map_question)
            .transpose()
            .map_err(Into::into)
    }

    async fn open_question(
        &self,
        session: &ArchitectureSessionId,
    ) -> PortResult<Option<ArchitectureQuestion>> {
        let row = sqlx::query(&format!(
            "SELECT {QUESTION_COLUMNS} FROM architecture_question
             WHERE session_id = ? AND status = 'open' LIMIT 1"
        ))
        .bind(session.as_str())
        .fetch_optional(self.pool())
        .await
        .map_err(StoreError::from)?;
        row.as_ref()
            .map(map_question)
            .transpose()
            .map_err(Into::into)
    }

    async fn questions_for_session(
        &self,
        session: &ArchitectureSessionId,
    ) -> PortResult<Vec<ArchitectureQuestion>> {
        let rows = sqlx::query(&format!(
            "SELECT {QUESTION_COLUMNS} FROM architecture_question
             WHERE session_id = ? ORDER BY sequence ASC"
        ))
        .bind(session.as_str())
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        rows.iter()
            .map(map_question)
            .collect::<Result<Vec<_>, StoreError>>()
            .map_err(Into::into)
    }

    async fn save_question(&self, question: &ArchitectureQuestion) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO architecture_question
               (id, session_id, sequence, prompt, status, answer, answered_at)
             VALUES (?, ?, ?, ?, ?, ?, CASE WHEN ? = 'answered' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END)
             ON CONFLICT(id) DO UPDATE SET
               status = excluded.status,
               answer = excluded.answer,
               answered_at = excluded.answered_at",
        )
        .bind(question.id.as_str())
        .bind(question.session_id.as_str())
        .bind(i64::from(question.sequence))
        .bind(&question.prompt)
        .bind(question.status.as_str())
        .bind(&question.answer)
        .bind(question.status.as_str())
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }

    async fn save_turn(
        &self,
        session: &ArchitectureSession,
        changed_question: Option<&ArchitectureQuestion>,
        next_question: Option<&ArchitectureQuestion>,
    ) -> PortResult<()> {
        let mut transaction = self.pool().begin().await.map_err(StoreError::from)?;
        if let Some(question) = changed_question {
            sqlx::query(
                "UPDATE architecture_question SET
                   status = ?, answer = ?,
                   answered_at = CASE WHEN ? = 'answered' THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END
                 WHERE id = ?",
            )
            .bind(question.status.as_str())
            .bind(&question.answer)
            .bind(question.status.as_str())
            .bind(question.id.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(StoreError::from)?;
        }
        sqlx::query(
            "INSERT INTO architecture_session
               (id, project_id, status, phase, acp_session_id, failure_reason)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               status = excluded.status,
               phase = excluded.phase,
               acp_session_id = excluded.acp_session_id,
               failure_reason = excluded.failure_reason,
               updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(session.id.as_str())
        .bind(session.project_id.as_str())
        .bind(session.status.as_str())
        .bind(session.phase.as_str())
        .bind(&session.acp_session_id)
        .bind(&session.failure_reason)
        .execute(&mut *transaction)
        .await
        .map_err(StoreError::from)?;
        if let Some(question) = next_question {
            sqlx::query(
                "INSERT INTO architecture_question
                   (id, session_id, sequence, prompt, status, answer, answered_at)
                 VALUES (?, ?, ?, ?, ?, ?, NULL)",
            )
            .bind(question.id.as_str())
            .bind(question.session_id.as_str())
            .bind(i64::from(question.sequence))
            .bind(&question.prompt)
            .bind(question.status.as_str())
            .bind(&question.answer)
            .execute(&mut *transaction)
            .await
            .map_err(StoreError::from)?;
        }
        transaction.commit().await.map_err(StoreError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_fixtures;

    #[tokio::test]
    async fn session_and_question_round_trip_with_unique_active_guards() {
        let store = test_fixtures::store_with_project().await;
        let mut session = ArchitectureSession::new(
            ArchitectureSessionId::new("as1").unwrap(),
            test_fixtures::PROJECT.clone(),
        );
        session.attach_agent("acp:as1").unwrap();
        session.ask(ArchitecturePhase::DomainDiscovery).unwrap();
        ArchitectureSessionStore::save(&store, &session)
            .await
            .unwrap();

        let mut question = ArchitectureQuestion::new(
            ArchitectureQuestionId::new("aq1").unwrap(),
            session.id.clone(),
            1,
            "Quel problème doit disparaître ?",
        )
        .unwrap();
        store.save_question(&question).await.unwrap();

        assert_eq!(
            store
                .active_for_project(&test_fixtures::PROJECT)
                .await
                .unwrap(),
            Some(session.clone())
        );
        assert_eq!(
            store.open_question(&session.id).await.unwrap(),
            Some(question.clone())
        );

        question.answer("Le routage manuel entre agents").unwrap();
        store.save_question(&question).await.unwrap();
        session.receive_answer().unwrap();
        store.save(&session).await.unwrap();
        assert!(store.open_question(&session.id).await.unwrap().is_none());
        assert_eq!(
            store.questions_for_session(&session.id).await.unwrap(),
            vec![question]
        );

        let second = ArchitectureSession::new(
            ArchitectureSessionId::new("as2").unwrap(),
            test_fixtures::PROJECT.clone(),
        );
        assert!(ArchitectureSessionStore::save(&store, &second)
            .await
            .is_err());
    }
}
