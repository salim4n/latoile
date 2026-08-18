//! SQLite persistence — one module per aggregate, all SQL in LaToile lives
//! here and in the vault (architecture contract §1, guardian check #3).
//!
//! One `Store` holds the pool and implements every store trait from
//! `latoile_core::ports`; each aggregate's implementation is in its own file.
//! Rows are mapped to domain types by hand: core structs expose their fields,
//! so rehydration is a struct literal — the state machines stay the only way
//! to *change* state, and the CHECK constraints guarantee stored values are
//! ones the domain itself produced.
//!
//! Core entities carry no audit columns; `created_at`/`updated_at` are
//! maintained by SQLite (ISO-8601 TEXT) and never read back.

mod approval;
mod architecture;
mod architecture_draft;
mod conversation;
mod delivery;
mod event;
mod preview;
mod project;
mod role;
mod run;
mod setting;
mod spec;
mod task;

pub use approval::InboxApprovalRow;
pub use conversation::ProjectMessageRow;
pub use project::ProjectListRow;
pub use role::RoleRow;
pub use task::ProjectTaskRow;

use latoile_core::error::DomainError;
use latoile_core::ports::PortError;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

/// What the store reports. Mapped into the opaque `PortError` at the trait
/// boundary — internal chains are for logs, never clients (contract §5).
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// A stored value no longer maps to a domain type. Should be impossible
    /// thanks to CHECK constraints; means the file was edited outside the app.
    #[error("corrupt row: {0}")]
    CorruptRow(String),
}

impl From<DomainError> for StoreError {
    /// Rebuilding an id from a stored string failed → the row is corrupt.
    fn from(e: DomainError) -> Self {
        StoreError::CorruptRow(e.to_string())
    }
}

impl From<StoreError> for PortError {
    fn from(e: StoreError) -> Self {
        PortError(e.to_string())
    }
}

/// A stored string that is none of the enum's wire values.
pub(crate) fn unknown_variant(entity: &str, raw: &str) -> StoreError {
    StoreError::CorruptRow(format!("unknown {entity} value {raw:?}"))
}

/// The application database. Clone to share the pool.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open (creating if needed) and bring the schema up to date. Migrations
    /// are embedded and applied here — an upgrade is never something the user
    /// has to remember (contract §4).
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .map_err(|e| StoreError::CorruptRow(format!("invalid path: {e}")))?
            .create_if_missing(true)
            .foreign_keys(true)
            // readers don't block the writer, which matters while events append
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        sqlx::migrate!("../../migrations/app").run(&pool).await?;
        Ok(Self { pool })
    }

    /// In-memory, for tests. One connection: an in-memory database dies with
    /// its connection, and the pool must not open a second, empty one.
    pub async fn open_ephemeral() -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(|e| StoreError::CorruptRow(e.to_string()))?
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!("../../migrations/app").run(&pool).await?;
        Ok(Self { pool })
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Cheap readiness proof used by the open health endpoint. Migrations
    /// have already completed when this can be called, so a failed read means
    /// the process must not advertise a healthy database.
    pub async fn health(&self) -> Result<(), StoreError> {
        let value: i64 = sqlx::query_scalar("SELECT 1").fetch_one(&self.pool).await?;
        if value != 1 {
            return Err(StoreError::CorruptRow(
                "database readiness query returned an unexpected value".into(),
            ));
        }
        Ok(())
    }

    /// Produce a transactionally consistent, standalone SQLite file while
    /// the source uses WAL. `VACUUM INTO` never copies a half-checkpointed
    /// database and refuses to overwrite the destination.
    pub async fn backup_to(&self, destination: &Path) -> Result<(), StoreError> {
        if destination.exists() {
            return Err(StoreError::CorruptRow(format!(
                "backup destination already exists: {}",
                destination.display()
            )));
        }
        let destination = destination.to_str().ok_or_else(|| {
            StoreError::CorruptRow("backup destination is not valid UTF-8".into())
        })?;
        sqlx::query("VACUUM INTO ?")
            .bind(destination)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Full SQLite structural check. Restore runs this on a disposable copy
    /// before any file is installed into the deployment home.
    pub async fn integrity_check(&self) -> Result<(), StoreError> {
        let rows = sqlx::query("PRAGMA integrity_check")
            .fetch_all(&self.pool)
            .await?;
        let findings = rows
            .iter()
            .map(|row| row.try_get::<String, _>(0))
            .collect::<Result<Vec<_>, _>>()?;
        if findings.as_slice() != ["ok"] {
            return Err(StoreError::CorruptRow(format!(
                "SQLite integrity check failed: {}",
                findings.join("; ")
            )));
        }
        Ok(())
    }
}

/// Shared fixtures so each aggregate's tests stay focused on its own table.
#[cfg(test)]
pub(crate) mod test_fixtures {
    use super::*;
    use latoile_core::ports::{ProjectStore, RunStore, SpecStore, TaskStore};
    use latoile_core::{
        Project, ProjectId, RoleId, Run, RunId, SpecVersion, SpecVersionId, Task, TaskId,
    };
    use std::sync::LazyLock;

    pub(crate) static PROJECT: LazyLock<ProjectId> =
        LazyLock::new(|| ProjectId::new("p1").unwrap());
    pub(crate) const SPEC: &str = "s1";

    pub(crate) async fn store_with_project() -> Store {
        let store = Store::open_ephemeral().await.unwrap();
        let project = Project::new(
            PROJECT.clone(),
            "Mon App",
            "mon-app",
            "salim4n/mon-app",
            "work",
            "/srv/latoile/mon-app",
            "pnpm dev --port $PORT",
        )
        .unwrap();
        ProjectStore::save(&store, &project).await.unwrap();
        store
    }

    pub(crate) async fn store_with_approved_spec() -> Store {
        let store = store_with_project().await;
        let mut spec = SpecVersion::new(
            SpecVersionId::new(SPEC).unwrap(),
            PROJECT.clone(),
            1,
            "design/",
            None,
        )
        .unwrap();
        spec.approve().unwrap();
        SpecStore::save(&store, &spec).await.unwrap();
        store
    }

    /// A task bound to the approved spec, still in `ready`.
    pub(crate) async fn store_with_task() -> (Store, TaskId) {
        let store = store_with_approved_spec().await;
        let mut task = Task::new(
            TaskId::new("t1").unwrap(),
            PROJECT.clone(),
            RoleId::new("frontend").unwrap(),
            "Page de connexion",
            "Formulaire email + mot de passe",
            0,
        )
        .unwrap();
        task.bind_spec(SpecVersionId::new(SPEC).unwrap());
        TaskStore::save(&store, &task).await.unwrap();
        (store, task.id)
    }

    /// A run (still `starting`) on the fixture task.
    pub(crate) async fn store_with_run() -> (Store, RunId) {
        let (store, task) = store_with_task().await;
        let run = Run::new(
            RunId::new("r1").unwrap(),
            task,
            RoleId::new("frontend").unwrap(),
            latoile_core::TriggeredBy::Manager,
        );
        RunStore::save(&store, &run).await.unwrap();
        (store, run.id)
    }
}

#[cfg(test)]
mod operational_tests {
    use super::*;
    use latoile_core::ports::ProjectStore;

    #[tokio::test]
    async fn vacuum_backup_is_consistent_and_refuses_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "latoile-store-backup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source_path = root.join("source.db");
        let backup_path = root.join("backup.db");
        let source = Store::open(&source_path).await.unwrap();
        let project = latoile_core::Project::new(
            latoile_core::ProjectId::new("p-backup").unwrap(),
            "Backup",
            "backup",
            "owner/backup",
            "work",
            "/srv/backup",
            "npm run dev -- --port $PORT",
        )
        .unwrap();
        ProjectStore::save(&source, &project).await.unwrap();

        source.health().await.unwrap();
        source.backup_to(&backup_path).await.unwrap();
        assert!(source.backup_to(&backup_path).await.is_err());

        let restored = Store::open(&backup_path).await.unwrap();
        restored.integrity_check().await.unwrap();
        assert_eq!(
            ProjectStore::get(&restored, &project.id)
                .await
                .unwrap()
                .unwrap(),
            project
        );

        drop(restored);
        drop(source);
        std::fs::remove_dir_all(root).ok();
    }
}
