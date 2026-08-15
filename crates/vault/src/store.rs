//! The secret store — the `SecretStore` port on top of the `secret` table.
//!
//! The rest of the app sees plaintext `get`/`put`; everything at rest is an
//! envelope produced by [`crate::crypto`]. Both stored columns are the
//! base64 of `nonce || ciphertext` — the table declares them TEXT, and base64
//! is the form that fits a TEXT column without a schema change.
//!
//! A value is never logged, never part of an error message, and never in a
//! `Debug` — the contract's §5, enforced by there simply being no code path
//! that formats one.

use crate::crypto::{RootKey, Sealed};
use crate::VaultError;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use latoile_core::ports::{PortResult, SecretStore};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use zeroize::Zeroizing;

/// The vault. Clone to share the pool; the root key is cloned with it.
#[derive(Clone)]
pub struct Vault {
    pool: SqlitePool,
    root: RootKey,
}

impl Vault {
    /// Wrap an existing, already-migrated pool — the app's own connection to
    /// the same database, for example.
    pub fn new(pool: SqlitePool, root: RootKey) -> Self {
        Self { pool, root }
    }

    /// Open the vault's own pool to the database file and bring the schema up
    /// to date. WAL allows this pool to coexist with the app's (contract §4:
    /// migrations are embedded and idempotent).
    pub async fn open(path: &Path, root: RootKey) -> Result<Self, VaultError> {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .map_err(|e| VaultError::KeyUnavailable(format!("invalid db path: {e}")))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await?;
        sqlx::migrate!("../../migrations/app").run(&pool).await?;
        Ok(Self { pool, root })
    }

    /// In-memory, for tests.
    pub async fn open_ephemeral(root: RootKey) -> Result<Self, VaultError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!("../../migrations/app").run(&pool).await?;
        Ok(Self { pool, root })
    }

    /// Like `get`, but a missing secret is an error with a name on it — for
    /// call sites where absence is a misconfiguration, not a normal state.
    pub async fn require(&self, name: &str) -> Result<String, VaultError> {
        self.get_plain(name)
            .await?
            .ok_or_else(|| VaultError::NotFound(name.to_string()))
    }

    /// Store a value, replacing whatever was there. The replacement re-seals
    /// under a fresh per-secret key and stamps `rotated_at` — an overwrite IS
    /// a rotation.
    async fn put_plain(&self, name: &str, value: &str) -> Result<(), VaultError> {
        let sealed = self.root.seal(name, value.as_bytes())?;
        sqlx::query(
            "INSERT INTO secret (name, ciphertext, wrapped_key)
             VALUES (?, ?, ?)
             ON CONFLICT(name) DO UPDATE SET
               ciphertext = excluded.ciphertext,
               wrapped_key = excluded.wrapped_key,
               rotated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(name)
        .bind(B64.encode(&sealed.ciphertext))
        .bind(B64.encode(&sealed.wrapped_key))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Read a value. `None` means nothing is stored — a normal state, not an
    /// error. A value that is there but doesn't verify IS an error.
    async fn get_plain(&self, name: &str) -> Result<Option<String>, VaultError> {
        let Some(row) =
            sqlx::query("SELECT ciphertext, wrapped_key FROM secret WHERE name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?
        else {
            return Ok(None);
        };

        let sealed = Sealed {
            ciphertext: decode_blob(&row.try_get::<String, _>("ciphertext")?)?,
            wrapped_key: decode_blob(&row.try_get::<String, _>("wrapped_key")?)?,
        };
        let opened = self.root.open(name, &sealed)?;
        let value = Zeroizing::new(
            String::from_utf8(opened.to_vec())
                .map_err(|_| VaultError::KeyUnavailable("a stored secret is not text".into()))?,
        );
        Ok(Some(value.to_string()))
    }
}

fn decode_blob(text: &str) -> Result<Vec<u8>, VaultError> {
    B64.decode(text).map_err(|_| VaultError::DecryptionFailed)
}

impl SecretStore for Vault {
    async fn get(&self, name: &str) -> PortResult<Option<String>> {
        self.get_plain(name).await.map_err(Into::into)
    }

    async fn put(&self, name: &str, value: &str) -> PortResult<()> {
        self.put_plain(name, value).await.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn vault() -> Vault {
        Vault::open_ephemeral(RootKey::generate()).await.unwrap()
    }

    #[tokio::test]
    async fn a_stored_secret_comes_back() {
        let v = vault().await;
        v.put("github", "gho_a-token").await.unwrap();
        assert_eq!(v.get("github").await.unwrap().as_deref(), Some("gho_a-token"));
    }

    #[tokio::test]
    async fn nothing_stored_is_not_an_error() {
        let v = vault().await;
        assert_eq!(v.get("github").await.unwrap(), None);
        let err = v.require("github").await.unwrap_err();
        assert!(matches!(err, VaultError::NotFound(name) if name == "github"));
    }

    #[tokio::test]
    async fn overwriting_replaces_the_value_and_stamps_rotation() {
        let v = vault().await;
        v.put("github", "first").await.unwrap();
        v.put("github", "second").await.unwrap();

        assert_eq!(v.get("github").await.unwrap().as_deref(), Some("second"));
        let row = sqlx::query("SELECT rotated_at FROM secret WHERE name = 'github'")
            .fetch_one(&v.pool)
            .await
            .unwrap();
        assert!(
            row.try_get::<Option<String>, _>("rotated_at").unwrap().is_some(),
            "an overwrite is a rotation"
        );
    }

    #[tokio::test]
    async fn another_root_key_reads_nothing() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations/app").run(&pool).await.unwrap();

        let ours = Vault::new(pool.clone(), RootKey::generate());
        ours.put("github", "a-token").await.unwrap();

        let theirs = Vault::new(pool, RootKey::generate());
        let err = theirs.get("github").await.unwrap_err();
        assert!(
            matches!(err, latoile_core::ports::PortError(_)),
            "mapped to the opaque port error: {err:?}"
        );
    }

    #[tokio::test]
    async fn the_value_is_not_in_the_row() {
        let v = vault().await;
        v.put("github", "gho_supersecret").await.unwrap();

        let row = sqlx::query("SELECT ciphertext, wrapped_key FROM secret")
            .fetch_one(&v.pool)
            .await
            .unwrap();
        for column in ["ciphertext", "wrapped_key"] {
            let blob: String = row.try_get(column).unwrap();
            assert!(
                !blob.contains("gho_supersecret"),
                "the token is readable in {column}"
            );
        }
    }

    /// The associated data's promise: a row renamed in the database is not a
    /// credential for its new name.
    #[tokio::test]
    async fn a_row_moved_to_another_name_does_not_open() {
        let v = vault().await;
        v.put("github", "a-token").await.unwrap();
        sqlx::query("UPDATE secret SET name = 'anthropic' WHERE name = 'github'")
            .execute(&v.pool)
            .await
            .unwrap();
        assert!(v.get("anthropic").await.is_err());
    }

    /// The vault can share the database file with the app: two pools, one
    /// file, WAL. Covered here with a tempdir so `cargo test` stays hermetic.
    #[tokio::test]
    async fn two_pools_share_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("latoile.db");
        let root = RootKey::generate();

        let first = Vault::open(&path, root.clone()).await.unwrap();
        first.put("github", "a-token").await.unwrap();

        let second = Vault::open(&path, root).await.unwrap();
        assert_eq!(second.get("github").await.unwrap().as_deref(), Some("a-token"));
    }
}
